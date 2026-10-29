use std::str::FromStr;

use nutype::nutype;

use super::{LocalPort, ValueParseError};

/// The one scheme a probe speaks: probes carry no TLS.
const HTTP_SCHEME: &str = "http://";
/// The hosts a URL may name, so a probe never leaves this machine.
const LOOPBACK_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "[::1]"];
/// Characters that end a URL's authority.
const AUTHORITY_END: [char; 3] = ['/', '?', '#'];
/// Opens a bracketed IPv6 host.
const IPV6_OPEN: char = '[';
/// Closes a bracketed IPv6 host.
const IPV6_CLOSE: char = ']';
/// Separates a host from its port.
const PORT_SEPARATOR: char = ':';
/// Marks credentials in an authority, which a probe URL never carries.
const USERINFO_MARK: char = '@';
/// A path separator some parsers read as `/`, refused so every parser agrees.
const BACKSLASH: char = '\\';
/// The form a URL is written in.
const URL_FORM: &str = "an http URL on localhost, 127.0.0.1 or [::1]";

/// Whether `url` is an `http` URL whose host is loopback, with no
/// credentials and at most a non-zero port. Anything another parser could
/// read differently (whitespace, control characters, a backslash) is refused,
/// so the host checked here is the host a probe connects to.
fn is_loopback_url(url: &str) -> bool {
    if url.chars().any(|character| {
        character.is_whitespace() || character.is_control() || character == BACKSLASH
    }) {
        return false;
    }
    let Some((host, port)) = host_and_port(url) else {
        return false;
    };
    let port_ok = port.is_empty() || port_number(port).is_some_and(|port| port > 0);
    port_ok
        && LOOPBACK_HOSTS
            .iter()
            .any(|loopback| host.eq_ignore_ascii_case(loopback))
}

/// The host of an `http` URL, and its `:port` suffix (empty when none), or
/// none for another scheme, credentials or an unclosed IPv6 host.
fn host_and_port(url: &str) -> Option<(&str, &str)> {
    let rest = url
        .get(..HTTP_SCHEME.len())
        .filter(|scheme| scheme.eq_ignore_ascii_case(HTTP_SCHEME))
        .map(|_| &url[HTTP_SCHEME.len()..])?;
    let authority = rest.split(AUTHORITY_END).next().unwrap_or_default();
    if authority.contains(USERINFO_MARK) {
        return None;
    }
    match authority.strip_prefix(IPV6_OPEN) {
        Some(_) => authority
            .find(IPV6_CLOSE)
            .map(|close| authority.split_at(close + IPV6_CLOSE.len_utf8())),
        None => Some(
            authority
                .find(PORT_SEPARATOR)
                .map_or((authority, ""), |at| authority.split_at(at)),
        ),
    }
}

/// The number of a `:port` suffix, or none when it is not one.
fn port_number(suffix: &str) -> Option<u16> {
    suffix
        .strip_prefix(PORT_SEPARATOR)
        .filter(|digits| digits.bytes().all(|byte| byte.is_ascii_digit()))
        .and_then(|digits| digits.parse().ok())
}

/// An `http` URL on this machine's loopback interface, which a readiness
/// check or wait probes. Restricted to loopback so a probe cannot reach
/// another host on an agent's behalf.
#[nutype(
    sanitize(trim),
    validate(predicate = |url: &str| is_loopback_url(url)),
    derive(Debug, Clone, PartialEq, Eq, Hash, AsRef, Display, Serialize, Deserialize)
)]
pub struct LocalUrl(String);

impl LocalUrl {
    /// The port the URL names, when it names one.
    pub fn port(&self) -> Option<LocalPort> {
        host_and_port(self.as_ref())
            .and_then(|(_, suffix)| port_number(suffix))
            .and_then(|port| LocalPort::try_new(port).ok())
    }
}

impl FromStr for LocalUrl {
    type Err = ValueParseError;

    /// Reads a URL such as `http://localhost:3000/health`.
    ///
    /// # Errors
    /// Returns [`ValueParseError`] for a URL that is not `http` on loopback.
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::try_new(raw).map_err(|_| {
            ValueParseError::builder()
                .expected(URL_FORM)
                .found(raw)
                .build()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_http_urls_are_accepted() {
        for raw in [
            "http://localhost:3000/health",
            "http://localhost",
            "HTTP://LocalHost:8080",
            "http://127.0.0.1:5173/?q=1#top",
            "http://[::1]:3000/",
            "  http://localhost:3000  ",
        ] {
            assert!(raw.parse::<LocalUrl>().is_ok(), "{raw}");
        }
        assert_eq!(
            " http://localhost:3000 "
                .parse::<LocalUrl>()
                .unwrap()
                .as_ref(),
            "http://localhost:3000"
        );
    }

    #[test]
    fn the_port_is_read_when_named() {
        let port = |raw: &str| {
            raw.parse::<LocalUrl>()
                .unwrap()
                .port()
                .map(LocalPort::into_inner)
        };
        assert_eq!(port("http://localhost:3000/health"), Some(3000));
        assert_eq!(port("http://[::1]:5173"), Some(5173));
        assert_eq!(port("http://127.0.0.1/"), None);
    }

    /// A probe must not become a way to reach another host: other hosts,
    /// credentials that move the real host, other schemes and anything two
    /// parsers could read differently are refused.
    #[test]
    fn urls_off_this_machine_or_ambiguous_are_refused() {
        for raw in [
            "https://localhost:3000",
            "ftp://localhost",
            "localhost:3000",
            "http://example.com",
            "http://localhost.example.com",
            "http://127.0.0.2:3000",
            "http://localhost@example.com/",
            "http://user:pass@localhost/",
            "http://example.com#@localhost",
            "http://localhost\\@example.com",
            "http://local host",
            "http://localhost:0",
            "http://localhost:65536",
            "http://localhost:",
            "http://localhost:30a",
            "http://[::1",
            "http://[::2]:80",
            "http://[::1]x",
            "http://",
            "",
        ] {
            assert!(raw.parse::<LocalUrl>().is_err(), "{raw}");
        }
    }
}
