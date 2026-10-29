//! One-shot TCP and HTTP probes of this machine's loopback interface, for
//! readiness checks and waits. Each call blocks for at most its timeout, so it
//! runs on a probe worker, never on the service loop.

use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpStream},
    time::Duration,
};

use ureq::{Agent, http::Uri};

use crate::domain::value::{LocalPort, LocalUrl};

/// Longest a TCP connect attempt waits on one address.
pub const PROBE_CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
/// Longest an HTTP probe waits for its whole response head.
pub const PROBE_HTTP_TIMEOUT: Duration = Duration::from_secs(2);
/// The first status a server error answers with, which a plain HTTP probe
/// does not count as up.
const SERVER_ERROR: u16 = 500;
/// The loopback addresses a port probe tries, in order.
const LOOPBACK: [IpAddr; 2] = [
    IpAddr::V4(Ipv4Addr::LOCALHOST),
    IpAddr::V6(Ipv6Addr::LOCALHOST),
];
/// The hosts an HTTP probe may connect to, as the request's parsed URI names
/// them.
const LOOPBACK_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "[::1]"];
/// Marks credentials in an authority.
const USERINFO_MARK: char = '@';
/// The one scheme an HTTP probe speaks.
const HTTP_SCHEME: &str = "http";

/// Whether something accepts a TCP connection on `port`, on `127.0.0.1` or
/// else `::1`.
pub fn probe_port(port: LocalPort) -> bool {
    LOOPBACK.iter().any(|ip| {
        TcpStream::connect_timeout(
            &SocketAddr::new(*ip, port.into_inner()),
            PROBE_CONNECT_TIMEOUT,
        )
        .is_ok()
    })
}

/// Whether a GET of `url` answers with `status`, or with any status below 500
/// when none is given. Redirects are not followed and no proxy is used, so
/// the request never leaves the loopback host the URL names.
pub fn probe_http(url: &LocalUrl, status: Option<u16>) -> bool {
    let Some(uri) = loopback_uri(url) else {
        return false;
    };
    let agent: Agent = Agent::config_builder()
        .proxy(None)
        .max_redirects(0)
        .http_status_as_error(false)
        .timeout_connect(Some(PROBE_CONNECT_TIMEOUT))
        .timeout_global(Some(PROBE_HTTP_TIMEOUT))
        .build()
        .into();
    agent.get(uri).call().is_ok_and(|response| {
        let answered = response.status().as_u16();
        status.map_or(answered < SERVER_ERROR, |wanted| answered == wanted)
    })
}

/// `url` as the HTTP client parses it, when that parse also names a loopback
/// host with no credentials: a second check, so a URL the domain and the
/// client read differently is never sent.
fn loopback_uri(url: &LocalUrl) -> Option<Uri> {
    let uri: Uri = url.as_ref().parse().ok()?;
    let authority = uri.authority()?;
    let loopback = uri.scheme_str() == Some(HTTP_SCHEME)
        && !authority.as_str().contains(USERINFO_MARK)
        && LOOPBACK_HOSTS
            .iter()
            .any(|host| authority.host().eq_ignore_ascii_case(host));
    loopback.then_some(uri)
}

#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
        thread,
    };

    use super::*;

    /// A port nothing listens on: bound, read, then released.
    fn free_port() -> u16 {
        TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    /// Serves one HTTP request on `listener` with `status`.
    fn answer_once(listener: TcpListener, status: u16) -> thread::JoinHandle<()> {
        thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap() > 0 && line != "\r\n" {
                line.clear();
            }
            let mut stream = stream;
            write!(
                stream,
                "HTTP/1.1 {status} X\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
            )
            .unwrap();
        })
    }

    #[test]
    fn a_port_probe_passes_only_while_something_listens() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = LocalPort::try_new(listener.local_addr().unwrap().port()).unwrap();
        assert!(probe_port(port));
        drop(listener);
        assert!(!probe_port(LocalPort::try_new(free_port()).unwrap()));
    }

    #[test]
    fn an_http_probe_counts_statuses_below_500_or_the_one_asked_for() {
        for (status, wanted, passes) in [
            (200, None, true),
            (404, None, true),
            (302, None, true),
            (503, None, false),
            (200, Some(200), true),
            (204, Some(200), false),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = answer_once(listener, status);
            let url: LocalUrl = format!("http://127.0.0.1:{port}/health").parse().unwrap();
            assert_eq!(probe_http(&url, wanted), passes, "{status} {wanted:?}");
            server.join().unwrap();
        }
    }

    #[test]
    fn an_http_probe_of_a_closed_port_fails() {
        let url: LocalUrl = format!("http://localhost:{}/", free_port())
            .parse()
            .unwrap();
        assert!(!probe_http(&url, None));
    }
}
