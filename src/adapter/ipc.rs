#[cfg(not(any(target_os = "linux", target_os = "android")))]
use std::os::fd::AsRawFd;
use std::{
    fs,
    io::{self, BufRead, BufReader, Read, Write},
    os::unix::net::{SocketAddr, UnixListener, UnixStream},
    path::Path,
    thread,
    time::Duration,
};

use crossbeam_channel::bounded;
#[cfg(any(target_os = "linux", target_os = "android"))]
use rustix::net::sockopt::socket_peercred;
use rustix::process::{Uid, geteuid};
use serde::{Serialize, de::DeserializeOwned};

/// Longest a peer may take to send its request line, or to read the response,
/// before the connection is abandoned. Bounds a client that connects and then
/// stalls, so it cannot hold a server thread forever.
const PEER_TIMEOUT: Duration = Duration::from_secs(10);
/// Largest request accepted, in bytes. Caps server-side allocation so one peer
/// cannot force unbounded memory with an endless line.
const MAX_REQUEST_BYTES: u64 = 1 << 20;

/// Whether `path` fits in a Unix socket address, whose `sun_path` holds
/// only about a hundred bytes.
pub fn fits_a_socket(path: &Path) -> bool {
    SocketAddr::from_pathname(path).is_ok()
}

/// Binds a listener at `path`, creating its parent directory.
///
/// A socket file that another live server is serving is never stolen: the
/// path is probed first, and a successful connect means an owner exists. Only
/// a stale file left by a crashed run (nothing accepting) is cleared and
/// rebound.
///
/// # Errors
/// Returns [`io::ErrorKind::AddrInUse`] when another server already serves
/// the socket, or an [`io::Error`] if the directory or socket cannot be
/// created.
pub fn bind(path: &Path) -> io::Result<UnixListener> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    match UnixStream::connect(path) {
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!(
                    "another muster process already serves the socket {}",
                    path.display()
                ),
            ));
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => {},
        // The file exists but nothing accepts: stale, safe to replace.
        Err(_) => match fs::remove_file(path) {
            Ok(()) => {},
            Err(error) if error.kind() == io::ErrorKind::NotFound => {},
            Err(error) => return Err(error),
        },
    }
    UnixListener::bind(path)
}

/// Connects to the socket at `path`, giving up after `timeout` so a listener
/// whose backlog is full cannot stall the caller. The attempt runs on a
/// helper thread, which a stalled connect leaves to finish on its own. A
/// peer run by another user is refused, as [`connect`] does.
///
/// # Errors
/// Returns the connect error, one of kind [`io::ErrorKind::TimedOut`] when
/// the connection was not made in time, or the refusal of a foreign peer.
pub fn connect_within(path: &Path, timeout: Duration) -> io::Result<UnixStream> {
    let (sender, receiver) = bounded(1);
    let path = path.to_path_buf();
    thread::spawn(move || {
        let _ = sender.send(connect(&path));
    });
    receiver
        .recv_timeout(timeout)
        .unwrap_or_else(|_| Err(io::Error::from(io::ErrorKind::TimedOut)))
}

/// Connects to the socket at `path` and refuses the connection unless the
/// process serving it runs as this user, so a socket another user planted
/// never hears a request.
///
/// # Errors
/// Returns the connect error, or one of kind
/// [`io::ErrorKind::PermissionDenied`] naming the socket's directory when
/// another user serves it.
pub fn connect(path: &Path) -> io::Result<UnixStream> {
    let stream = UnixStream::connect(path)?;
    own_peer(peer_uid(&stream)?, geteuid(), path)?;
    Ok(stream)
}

/// Refuses a peer whose user `peer` is not `own`, naming the directory of
/// the socket at `path`.
///
/// # Errors
/// Returns an error of kind [`io::ErrorKind::PermissionDenied`] when the
/// users differ.
fn own_peer(peer: Uid, own: Uid, path: &Path) -> io::Result<()> {
    if peer == own {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!(
            "the socket directory {} holds a socket served by another user (uid {}); muster will not talk to it",
            path.parent().unwrap_or(path).display(),
            peer.as_raw()
        ),
    ))
}

/// The user running the process at the other end of `stream`.
///
/// # Errors
/// Returns the failure to read the peer's credentials.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn peer_uid(stream: &UnixStream) -> io::Result<Uid> {
    Ok(socket_peercred(stream)?.uid)
}

/// The user running the process at the other end of `stream`.
///
/// # Errors
/// Returns the failure to read the peer's credentials.
#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn peer_uid(stream: &UnixStream) -> io::Result<Uid> {
    let mut uid = 0;
    let mut gid = 0;
    // SAFETY: the descriptor is a connected Unix socket `stream` keeps open
    // for the call, and both out-pointers name live locals of the right type.
    let status = unsafe { libc::getpeereid(stream.as_raw_fd(), &raw mut uid, &raw mut gid) };
    if status != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Uid::from_raw(uid))
}

/// Removes a socket file, ignoring an already-absent path. Called on clean
/// shutdown so sockets do not accumulate in the state directory.
pub fn unlink(path: &Path) {
    let _ = fs::remove_file(path);
}

/// Sends `request` to the socket at `path` and returns its response, one
/// newline-delimited JSON message each way, giving up once the response has
/// not arrived within `reply_timeout`, so a wedged server cannot hang its
/// client.
///
/// # Errors
/// Returns an [`io::Error`] if the socket cannot be reached (no server
/// running), another user serves it, or the exchange fails, or one of kind
/// [`io::ErrorKind::WouldBlock`] or [`io::ErrorKind::TimedOut`] when the
/// response is late.
pub fn request_within<Req, Resp>(
    path: &Path,
    request: &Req,
    reply_timeout: Duration,
) -> io::Result<Resp>
where
    Req: Serialize,
    Resp: DeserializeOwned,
{
    let stream = connect(path)?;
    stream.set_read_timeout(Some(reply_timeout))?;
    stream.set_write_timeout(Some(reply_timeout))?;
    exchange(stream, request)
}

/// Writes `request` on `stream` as one line and reads one response line.
///
/// # Errors
/// Returns an [`io::Error`] when the exchange fails or the response does not
/// parse.
fn exchange<Req, Resp>(stream: UnixStream, request: &Req) -> io::Result<Resp>
where
    Req: Serialize,
    Resp: DeserializeOwned,
{
    let encoded = serde_json::to_string(request).map_err(io::Error::other)?;
    (&stream).write_all(encoded.as_bytes())?;
    (&stream).write_all(b"\n")?;
    (&stream).flush()?;
    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "the peer closed the connection without answering",
        ));
    }
    serde_json::from_str(line.trim_end()).map_err(io::Error::other)
}

/// Reads one request from `stream`, hands it to `handle`, and writes the response
/// back as one newline-delimited JSON message.
///
/// # Errors
/// Returns an [`io::Error`] if the request cannot be read or parsed or the
/// response cannot be written.
pub fn serve_connection<Req, Resp>(
    stream: UnixStream,
    handle: impl FnOnce(Req) -> Resp,
) -> io::Result<()>
where
    Req: DeserializeOwned,
    Resp: Serialize,
{
    // A peer that connects and then stalls must not hold this thread forever,
    // and must not be able to force unbounded allocation with an endless line.
    stream.set_read_timeout(Some(PEER_TIMEOUT))?;
    stream.set_write_timeout(Some(PEER_TIMEOUT))?;
    let mut reader = BufReader::new((&stream).take(MAX_REQUEST_BYTES));
    let mut line = String::new();
    let read = reader.read_line(&mut line)?;
    if read == 0 || line.trim().is_empty() {
        return Ok(());
    }
    if !line.ends_with('\n') {
        return Err(io::Error::other(
            "request exceeded the maximum frame size before a newline",
        ));
    }
    let request: Req = serde_json::from_str(line.trim_end()).map_err(io::Error::other)?;
    let response = handle(request);
    let encoded = serde_json::to_string(&response).map_err(io::Error::other)?;
    (&stream).write_all(encoded.as_bytes())?;
    (&stream).write_all(b"\n")?;
    (&stream).flush()
}

#[cfg(test)]
mod tests {
    use std::{env, path::PathBuf, thread};

    use uuid::Uuid;

    use super::*;
    use crate::adapter::{
        ping::Ping,
        pong::Pong,
        test_wait::{REPLY, wait_until},
    };

    /// A socket path short enough for every platform's `sun_path` cap. macOS
    /// resolves `env::temp_dir()` to a long `/var/folders/...` path that breaches
    /// the 104-byte limit, so these tests bind under `/tmp` directly.
    fn short_socket_path() -> PathBuf {
        let unique = Uuid::new_v4().simple().to_string();
        Path::new("/tmp").join(format!("mstr-{}.sock", &unique[..8]))
    }

    /// A request round-trips through the listener to a handler and back.
    #[test]
    fn a_request_round_trips_through_the_socket() {
        let path = short_socket_path();
        let listener = bind(&path).unwrap();

        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve_connection(stream, |request: Ping| {
                assert_eq!(request, Ping::Ping);
                Pong::Pong(42)
            })
            .unwrap();
        });

        let response: Pong = request_within(&path, &Ping::Ping, REPLY).unwrap();
        assert_eq!(response, Pong::Pong(42));
        server.join().unwrap();
        unlink(&path);
    }

    /// A live server's socket is never stolen by a second instance, but a
    /// stale file left by a crashed run is reclaimed.
    #[test]
    fn bind_refuses_a_live_socket_and_reclaims_a_stale_one() {
        let path = short_socket_path();
        let live = bind(&path).unwrap();
        let stolen = bind(&path);
        assert_eq!(
            stolen.unwrap_err().kind(),
            io::ErrorKind::AddrInUse,
            "a second instance must not unlink the first's socket"
        );

        // Drop the listener: the file remains but nothing accepts, so the next
        // bind may reclaim it. A child another test thread forked meanwhile
        // holds a copy of the listener until it execs, so wait for the last
        // copy to close before calling the socket stale.
        drop(live);
        assert!(path.exists(), "the stale socket file is still on disk");
        wait_until("the dropped listener's last copy to close", || {
            UnixStream::connect(&path).is_err()
        });
        bind(&path).expect("a stale socket is reclaimed");
        unlink(&path);
    }

    /// A peer that floods bytes without a newline is cut off at the frame cap
    /// instead of growing the server's memory without bound.
    #[test]
    fn an_oversized_request_is_refused() {
        let path = short_socket_path();
        let listener = bind(&path).unwrap();

        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve_connection(stream, |_: Ping| Pong::Pong(0))
        });

        let stream = UnixStream::connect(&path).unwrap();
        let flood = vec![b'x'; (MAX_REQUEST_BYTES as usize) + 1024];
        // The peer never sends a newline; the write may fail once the server
        // gives up, which is itself the cap working.
        let _ = (&stream).write_all(&flood);
        assert!(
            server.join().unwrap().is_err(),
            "the server refuses a frame that exceeds the cap"
        );
        unlink(&path);
    }

    /// A listener run by this user is reached, and a peer of any other user
    /// is refused, naming the socket's directory.
    #[test]
    fn only_a_peer_of_this_user_is_trusted() {
        let path = short_socket_path();
        let listener = bind(&path).unwrap();

        assert!(connect(&path).is_ok());
        let own = geteuid();
        let foreign = Uid::from_raw(own.as_raw().wrapping_add(1));
        let error = own_peer(foreign, own, &path).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains("/tmp "), "{error}");
        drop(listener);
        unlink(&path);
    }

    /// Connecting when nothing listens is an error the caller reads as "no
    /// server running".
    #[test]
    fn connecting_with_no_listener_errors() {
        let path = env::temp_dir().join(format!("muster-absent-{}.sock", Uuid::new_v4()));
        let result: io::Result<Pong> = request_within(&path, &Ping::Ping, REPLY);
        assert!(result.is_err());
    }
}
