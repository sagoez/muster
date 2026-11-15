//! The head of a reply a test client read, and the GET it sends, shared by
//! the whole-reply client and the event stream reader.

use getset::Getters;
use httparse::{EMPTY_HEADER, Status};
use typed_builder::TypedBuilder;

/// Most headers a reply is parsed with.
const MAX_REPLY_HEADERS: usize = 32;
/// What ends a request's line, and its head after the last header.
const LINE_END: &str = "\r\n";

/// A reply's status and headers.
#[derive(Clone, Debug, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ReplyHead {
    /// The status code.
    status: u16,
    /// Every header, in order, with its name as sent.
    headers: Vec<(String, String)>,
}

impl ReplyHead {
    /// The head `bytes` start with, and how many bytes it took; none until
    /// they hold a whole one.
    ///
    /// # Panics
    /// When `bytes` start with something that is no HTTP response head.
    pub fn parse(bytes: &[u8]) -> Option<(Self, usize)> {
        let mut headers = [EMPTY_HEADER; MAX_REPLY_HEADERS];
        let mut response = httparse::Response::new(&mut headers);
        let parsed = response.parse(bytes).unwrap_or_else(|error| {
            panic!(
                "not a response head ({error}): {:?}",
                String::from_utf8_lossy(bytes)
            )
        });
        let Status::Complete(length) = parsed else {
            return None;
        };
        let head = Self::builder()
            .status(response.code.unwrap())
            .headers(
                response
                    .headers
                    .iter()
                    .map(|header| {
                        (
                            header.name.to_string(),
                            String::from_utf8_lossy(header.value).into_owned(),
                        )
                    })
                    .collect(),
            )
            .build();
        Some((head, length))
    }

    /// The first value of the header `name`, whatever its case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(sent, _)| sent.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// A GET for `target` naming `127.0.0.1:<port>` as its host, with `headers`
/// added.
pub fn get_request(port: u16, target: &str, headers: &[(&str, &str)]) -> String {
    let mut raw = format!("GET {target} HTTP/1.1{LINE_END}Host: 127.0.0.1:{port}{LINE_END}");
    for (name, value) in headers {
        raw.push_str(&format!("{name}: {value}{LINE_END}"));
    }
    raw.push_str(LINE_END);
    raw
}
