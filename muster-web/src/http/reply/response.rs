use std::{
    borrow::Cow,
    fmt::Write as _,
    io::{self, Write},
};

use getset::Getters;
use typed_builder::TypedBuilder;

use super::{cache_policy::CachePolicy, status::Status};
use crate::http::access::gate::GET;

/// The page's Content Security Policy: everything from the view itself, no
/// inline script, inline styles allowed for the page's own animation.
pub const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'";
/// The media type of a refusal's body.
const PLAIN_TEXT: &str = "text/plain; charset=utf-8";
/// The media type of the API's answers.
const JSON: &str = "application/json";
/// The media type of the live stream.
const EVENT_STREAM: &str = "text/event-stream";
/// The protocol every response is written in.
const HTTP_VERSION: &str = "HTTP/1.1";
/// What ends a header line, and the head.
const LINE_END: &str = "\r\n";
/// The headers every response carries, besides the policy: no sniffing a
/// media type, no referrer leaving the page, and no framing.
const HARDENING: [(&str, &str); 3] = [
    ("X-Content-Type-Options", "nosniff"),
    ("Referrer-Policy", "no-referrer"),
    ("X-Frame-Options", "DENY"),
];
/// The policy header's name.
const CSP_HEADER: &str = "Content-Security-Policy";
/// The cache header's name.
const CACHE_HEADER: &str = "Cache-Control";
/// The media type header's name.
const CONTENT_TYPE_HEADER: &str = "Content-Type";
/// The body length header's name.
const CONTENT_LENGTH_HEADER: &str = "Content-Length";
/// The connection header's name.
const CONNECTION_HEADER: &str = "Connection";
/// Each connection carries one request.
const CONNECTION_CLOSE: &str = "close";
/// The allowed methods header's name, on a 405.
const ALLOW_HEADER: &str = "Allow";
/// The redirect target header's name, on a 303.
const LOCATION_HEADER: &str = "Location";

/// One response, followed by the connection's close: written whole at
/// once, or only its head for a stream whose body runs until the close.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Response {
    /// The status.
    status: Status,
    /// How long a browser may keep it.
    cache: CachePolicy,
    /// The body's media type, when it has a body.
    #[builder(default)]
    content_type: Option<&'static str>,
    /// The methods allowed, on a 405.
    #[builder(default)]
    allow: Option<&'static str>,
    /// Where a 303 sends the browser.
    #[builder(default)]
    location: Option<String>,
    /// The body.
    #[builder(default)]
    body: Cow<'static, [u8]>,
    /// Whether the body follows the head until the connection closes, so
    /// the head names no length.
    #[builder(default)]
    streamed: bool,
}

impl Response {
    /// A refusal with `status`, its reason as the body; a 405 names the one
    /// method allowed.
    pub fn refusal(status: Status) -> Self {
        Self::builder()
            .status(status)
            .cache(CachePolicy::NoStore)
            .content_type(Some(PLAIN_TEXT))
            .allow((status == Status::MethodNotAllowed).then_some(GET))
            .body(Cow::Borrowed(status.reason().as_bytes()))
            .build()
    }

    /// A 303 that sends the browser on to `location`, never kept.
    pub fn see_other(location: String) -> Self {
        Self::builder()
            .status(Status::SeeOther)
            .cache(CachePolicy::NoStore)
            .location(Some(location))
            .build()
    }

    /// A 200 of `body` as `content_type`, kept as `cache` says.
    pub fn ok(content_type: &'static str, cache: CachePolicy, body: Cow<'static, [u8]>) -> Self {
        Self::builder()
            .status(Status::Ok)
            .cache(cache)
            .content_type(Some(content_type))
            .body(body)
            .build()
    }

    /// `body`, JSON, answered with `status` and never kept.
    pub fn json(status: Status, body: Vec<u8>) -> Self {
        Self::builder()
            .status(status)
            .cache(CachePolicy::NoStore)
            .content_type(Some(JSON))
            .body(Cow::Owned(body))
            .build()
    }

    /// The head of a live stream of server-sent events, never kept, whose
    /// body runs until the connection closes.
    pub fn event_stream() -> Self {
        Self::builder()
            .status(Status::Ok)
            .cache(CachePolicy::NoStore)
            .content_type(Some(EVENT_STREAM))
            .streamed(true)
            .build()
    }

    /// Writes the head and body to `out`, and flushes it.
    ///
    /// # Errors
    /// Returns the failure of the write.
    pub fn write_to(&self, out: &mut dyn Write) -> io::Result<()> {
        let mut head = format!(
            "{HTTP_VERSION} {} {}{LINE_END}",
            self.status.code(),
            self.status.reason()
        );
        let mut header = |name: &str, value: &str| {
            let _ = write!(head, "{name}: {value}{LINE_END}");
        };
        header(CSP_HEADER, CONTENT_SECURITY_POLICY);
        for (name, value) in HARDENING {
            header(name, value);
        }
        header(CACHE_HEADER, self.cache.header_value());
        if let Some(content_type) = self.content_type {
            header(CONTENT_TYPE_HEADER, content_type);
        }
        if let Some(allow) = self.allow {
            header(ALLOW_HEADER, allow);
        }
        if let Some(location) = &self.location {
            header(LOCATION_HEADER, location);
        }
        if !self.streamed {
            header(CONTENT_LENGTH_HEADER, &self.body.len().to_string());
        }
        header(CONNECTION_HEADER, CONNECTION_CLOSE);
        head.push_str(LINE_END);
        out.write_all(head.as_bytes())?;
        out.write_all(&self.body)?;
        out.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The policy is the spec's string and allows no script beyond the
    /// view's own files.
    #[test]
    fn the_policy_is_the_specs_and_holds_no_script_src() {
        assert_eq!(
            CONTENT_SECURITY_POLICY,
            "default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'"
        );
        assert!(!CONTENT_SECURITY_POLICY.contains("script-src"));
    }

    /// A refusal carries the policy, the hardening headers, no-store and a
    /// close, and a 405 names GET.
    #[test]
    fn a_refusal_is_hardened() {
        let mut written = Vec::new();

        Response::refusal(Status::MethodNotAllowed)
            .write_to(&mut written)
            .unwrap();

        let text = String::from_utf8(written).unwrap();
        assert_eq!(
            text,
            "HTTP/1.1 405 Method Not Allowed\r\n\
             Content-Security-Policy: default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'\r\n\
             X-Content-Type-Options: nosniff\r\n\
             Referrer-Policy: no-referrer\r\n\
             X-Frame-Options: DENY\r\n\
             Cache-Control: no-store\r\n\
             Content-Type: text/plain; charset=utf-8\r\n\
             Allow: GET\r\n\
             Content-Length: 18\r\n\
             Connection: close\r\n\
             \r\n\
             Method Not Allowed"
        );
    }

    /// A stream's head is hardened, never kept, and names no length, so its
    /// body runs until the close.
    #[test]
    fn a_stream_head_names_no_length() {
        let mut written = Vec::new();

        Response::event_stream().write_to(&mut written).unwrap();

        let text = String::from_utf8(written).unwrap();
        assert!(text.starts_with("HTTP/1.1 200 OK\r\n"), "{text}");
        assert!(
            text.contains("Content-Type: text/event-stream\r\n"),
            "{text}"
        );
        assert!(text.contains("Cache-Control: no-store\r\n"), "{text}");
        assert!(text.contains("X-Frame-Options: DENY\r\n"), "{text}");
        assert!(text.ends_with("Connection: close\r\n\r\n"), "{text}");
        assert!(!text.contains("Content-Length"), "{text}");
    }
}
