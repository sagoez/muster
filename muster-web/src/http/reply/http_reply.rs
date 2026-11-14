//! A raw HTTP client for tests: it sends exactly the bytes a test writes,
//! so it can send what no well-behaved client would, and parses the reply
//! with `httparse`.

use std::{
    io::{Read, Write},
    net::{Ipv4Addr, TcpStream},
    time::Duration,
};

use getset::Getters;
use httparse::{EMPTY_HEADER, Status};
use typed_builder::TypedBuilder;

/// Longest a test waits for one whole reply.
const REPLY_BOUND: Duration = Duration::from_secs(30);
/// Most headers a reply is parsed with.
const MAX_REPLY_HEADERS: usize = 32;

/// One reply the server sent, read until it closed the connection.
#[derive(Clone, Debug, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct HttpReply {
    /// The status code.
    status: u16,
    /// Every header, in order, with its name as sent.
    headers: Vec<(String, String)>,
    /// The body.
    body: Vec<u8>,
}

impl HttpReply {
    /// The first value of the header `name`, whatever its case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(sent, _)| sent.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// The body as text.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// Sends `raw` to the server on `port` on loopback and reads its reply.
///
/// # Panics
/// When the connection fails or the reply is not a whole HTTP response.
pub fn exchange(port: u16, raw: &[u8]) -> HttpReply {
    let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
    stream.set_read_timeout(Some(REPLY_BOUND)).unwrap();
    stream.write_all(raw).unwrap();
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).unwrap();
    let mut headers = [EMPTY_HEADER; MAX_REPLY_HEADERS];
    let mut response = httparse::Response::new(&mut headers);
    let Ok(Status::Complete(head)) = response.parse(&bytes) else {
        panic!(
            "not a whole response: {:?}",
            String::from_utf8_lossy(&bytes)
        );
    };
    HttpReply::builder()
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
        .body(bytes[head..].to_vec())
        .build()
}

/// A GET for `target` naming `127.0.0.1:<port>` as its host, with `headers`
/// added, sent to the server on `port`.
///
/// # Panics
/// As [`exchange`].
pub fn get(port: u16, target: &str, headers: &[(&str, &str)]) -> HttpReply {
    let mut raw = format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n");
    for (name, value) in headers {
        raw.push_str(&format!("{name}: {value}\r\n"));
    }
    raw.push_str("\r\n");
    exchange(port, raw.as_bytes())
}
