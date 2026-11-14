use std::{
    io::{self, ErrorKind, Read},
    net::{Ipv4Addr, Shutdown, TcpListener, TcpStream},
    sync::{Arc, atomic::AtomicUsize},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use muster_domain::value::LocalPort;

use super::{connection_slot::ConnectionSlot, head_read::HeadRead, responder::Responder};
use crate::http::reply::{answer::Answer, response::Response, status::Status};

/// The acceptor thread's name.
const ACCEPTOR_THREAD: &str = "view-acceptor";
/// Each connection thread's name.
const CONNECTION_THREAD: &str = "view-connection";
/// How long writing a response may stall on a client that stopped reading.
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a failed accept, such as one out of file descriptors, waits
/// before the next, so it never spins.
const ACCEPT_PAUSE: Duration = Duration::from_millis(100);
/// How long a closing connection reads what the client still sends, so its
/// close is a clean one and the client reads the whole response.
const LINGER: Duration = Duration::from_secs(1);
/// Most bytes a closing connection reads and drops.
const MAX_LINGER_BYTES: usize = 64 * 1024;
/// Most bytes one read of a closing connection takes.
const LINGER_CHUNK: usize = 1024;

/// The view's HTTP server: a listener on loopback, an acceptor thread, and
/// one thread per connection, each carrying one request.
pub struct HttpServer {
    /// The bound listener.
    listener: TcpListener,
}

impl HttpServer {
    /// A server listening on `127.0.0.1`, on `port` or on a free port when
    /// none is given.
    ///
    /// # Errors
    /// Returns the failure to bind.
    pub fn bind(port: Option<LocalPort>) -> io::Result<Self> {
        let port = port.map_or(0, LocalPort::into_inner);
        Ok(Self {
            listener: TcpListener::bind((Ipv4Addr::LOCALHOST, port))?,
        })
    }

    /// The port the server listens on.
    ///
    /// # Errors
    /// Returns the failure to read the listener's address.
    pub fn port(&self) -> io::Result<LocalPort> {
        LocalPort::try_new(self.listener.local_addr()?.port()).map_err(io::Error::other)
    }

    /// Starts accepting on a thread of its own, each connection answered by
    /// `responder`; returns that thread, which runs for as long as the
    /// process does.
    ///
    /// # Errors
    /// Returns the failure to start the thread.
    pub fn start(self, responder: Arc<dyn Responder>) -> io::Result<JoinHandle<()>> {
        thread::Builder::new()
            .name(ACCEPTOR_THREAD.to_string())
            .spawn(move || accept(&self.listener, &responder))
    }
}

/// Accepts every connection on `listener`, serving each on a thread of its
/// own while a place is free and closing it at once otherwise.
fn accept(listener: &TcpListener, responder: &Arc<dyn Responder>) {
    let taken = Arc::new(AtomicUsize::new(0));
    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            thread::sleep(ACCEPT_PAUSE);
            continue;
        };
        let Some(slot) = ConnectionSlot::take(&taken) else {
            continue;
        };
        let responder = Arc::clone(responder);
        // A thread that cannot start drops its closure, and with it the
        // stream and the place.
        let _ = thread::Builder::new()
            .name(CONNECTION_THREAD.to_string())
            .spawn(move || {
                let _slot = slot;
                serve(stream, responder.as_ref());
            });
    }
}

/// Reads one request from `stream`, answers it, and closes the connection:
/// a whole response at once, or a stream for as long as it runs, each write
/// bounded by [`WRITE_TIMEOUT`]. A head too large or malformed is refused; a
/// client that sends no whole head in time is closed unanswered.
fn serve(mut stream: TcpStream, responder: &dyn Responder) {
    let answer = match HeadRead::from_stream(&mut stream) {
        HeadRead::Request(request) => responder.respond(&request),
        HeadRead::TooLarge => Response::refusal(Status::HeaderFieldsTooLarge).into(),
        HeadRead::Malformed => Response::refusal(Status::BadRequest).into(),
        HeadRead::Gone => return,
    };
    if stream.set_write_timeout(Some(WRITE_TIMEOUT)).is_err() {
        return;
    }
    match answer {
        Answer::Whole(response) => {
            if response.write_to(&mut stream).is_ok() {
                close(stream);
            }
        },
        Answer::Stream(streamed) => {
            streamed.stream(&mut stream);
            close(stream);
        },
    }
}

/// Ends the response and reads, for at most [`LINGER`] in all and at most
/// [`MAX_LINGER_BYTES`], what the client still sends, so closing with unread
/// bytes never resets the connection before the client has read the
/// response. The deadline is re-armed before each read, so a client that
/// trickles bytes cannot stretch it.
fn close(mut stream: TcpStream) {
    let _ = stream.shutdown(Shutdown::Write);
    let deadline = Instant::now() + LINGER;
    let mut chunk = [0; LINGER_CHUNK];
    let mut drained = 0;
    while drained < MAX_LINGER_BYTES {
        let Some(left) = deadline
            .checked_duration_since(Instant::now())
            .filter(|left| !left.is_zero())
        else {
            return;
        };
        if stream.set_read_timeout(Some(left)).is_err() {
            return;
        }
        match stream.read(&mut chunk) {
            Ok(0) => return,
            Ok(read) => drained += read,
            Err(error) if error.kind() == ErrorKind::Interrupted => {},
            Err(_) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::Write,
        net::TcpStream,
        time::{Duration, Instant},
    };

    use muster_os::test_wait::wait_until;

    use super::*;
    use crate::{
        floor::fixture::floor,
        http::{
            access::{gate::Gate, token::ViewToken},
            connection::{
                connection_slot::MAX_CONNECTIONS,
                head_read::{HEAD_TIMEOUT, MAX_HEAD_BYTES},
            },
            reply::http_reply::{HttpReply, exchange, get},
        },
        routes::testing::live_site,
    };

    /// Well past [`HEAD_TIMEOUT`], for a cut that must come.
    const CUT_BOUND: Duration = Duration::from_secs(8);
    /// Well under [`HEAD_TIMEOUT`], for a close that must be prompt.
    const PROMPT: Duration = Duration::from_secs(2);
    /// How long past [`LINGER`] a trickling client may stay open, for a
    /// loaded machine.
    const CLOSE_MARGIN: Duration = Duration::from_secs(2);
    /// The pause between a trickling client's bytes, well inside one read's
    /// wait.
    const TRICKLE: Duration = Duration::from_millis(100);

    /// A started server on a free port, answered by a fresh site over the
    /// fixture floor; returns its port.
    fn started() -> u16 {
        let server = HttpServer::bind(None).unwrap();
        let port = server.port().unwrap();
        let gate = Gate::builder()
            .port(port)
            .token(ViewToken::generate().unwrap())
            .build();
        let (site, _) = live_site(gate, floor());
        server.start(Arc::new(site)).unwrap();
        port.into_inner()
    }

    /// A connection to the server on `port`.
    fn connect(port: u16) -> TcpStream {
        TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap()
    }

    /// Whether the server closed `stream` without answering.
    fn closed_unanswered(stream: &mut TcpStream) -> bool {
        stream.set_read_timeout(Some(CUT_BOUND)).unwrap();
        let mut byte = [0; 1];
        matches!(stream.read(&mut byte), Ok(0) | Err(_))
    }

    /// A head over 8 KiB is answered 431, and the response survives the
    /// unread rest of it.
    #[test]
    fn an_oversized_head_is_431() {
        let port = started();
        let padding = "a".repeat(MAX_HEAD_BYTES + 1024);

        let reply = exchange(
            port,
            format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-Pad: {padding}\r\n\r\n")
                .as_bytes(),
        );

        assert_eq!(reply.status(), &431);
        assert_eq!(reply.header("Connection"), Some("close"));
    }

    /// A head with more headers than the parser holds is answered 431.
    #[test]
    fn too_many_headers_is_431() {
        let port = started();
        let headers = "X-A: 1\r\n".repeat(100);

        let reply = exchange(
            port,
            format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n{headers}\r\n").as_bytes(),
        );

        assert_eq!(reply.status(), &431);
    }

    /// A malformed request line is answered 400, hardened as every response
    /// is.
    #[test]
    fn a_malformed_request_line_is_400() {
        let port = started();

        let reply = exchange(port, b"NOT AN HTTP LINE\r\n\r\n");

        assert_eq!(reply.status(), &400);
        assert_eq!(reply.header("X-Frame-Options"), Some("DENY"));
    }

    /// A client that sends half a head and goes quiet is cut off once the
    /// head's time is up, unanswered.
    #[test]
    fn a_quiet_half_head_is_cut_off() {
        let port = started();
        let mut stream = connect(port);
        stream.write_all(b"GET / HTTP/1.1\r\nHost: 127").unwrap();
        let sent = Instant::now();

        assert!(closed_unanswered(&mut stream));

        let waited = sent.elapsed();
        assert!(
            waited >= HEAD_TIMEOUT - Duration::from_millis(100) && waited < CUT_BOUND,
            "cut after {waited:?}"
        );
    }

    /// Past the cap, a new connection is closed at accept while the others
    /// are held; once they go, a request is answered again.
    #[test]
    fn a_connection_past_the_cap_is_closed() {
        let port = started();
        let mut held = (0..MAX_CONNECTIONS)
            .map(|_| connect(port))
            .collect::<Vec<_>>();
        let mut over = connect(port);
        let tried = Instant::now();

        assert!(closed_unanswered(&mut over));
        assert!(
            tried.elapsed() < PROMPT,
            "closed after {:?}",
            tried.elapsed()
        );

        held.clear();
        wait_until("a freed place to answer", || {
            get(port, "/", &[]).status() == &403
        });
    }

    /// A client that keeps trickling bytes after its response, each well
    /// inside one read's wait, is still closed once the linger is up.
    #[test]
    fn a_trickling_client_is_closed_after_the_linger() {
        let port = started();
        let mut stream = connect(port);
        stream.set_read_timeout(Some(CUT_BOUND)).unwrap();
        stream
            .write_all(format!("GET / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").as_bytes())
            .unwrap();
        let mut reply = Vec::new();
        stream.read_to_end(&mut reply).unwrap();
        let answered = Instant::now();

        while stream.write_all(b"x").is_ok() {
            assert!(
                answered.elapsed() < LINGER + CLOSE_MARGIN,
                "still open after {:?}",
                answered.elapsed()
            );
            thread::sleep(TRICKLE);
        }

        assert!(reply.starts_with(b"HTTP/1.1 403"));
    }

    /// POST and HEAD are refused 405 naming GET, before any check of the
    /// token.
    #[test]
    fn only_get_is_served() {
        let port = started();

        for method in ["POST", "HEAD"] {
            let reply: HttpReply = exchange(
                port,
                format!(
                    "{method} / HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 0\r\n\r\n"
                )
                .as_bytes(),
            );
            assert_eq!(reply.status(), &405, "{method}");
            assert_eq!(reply.header("Allow"), Some("GET"), "{method}");
        }
    }
}
