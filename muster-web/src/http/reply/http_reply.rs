//! A raw HTTP client for tests: it sends exactly the bytes a test writes,
//! so it can send what no well-behaved client would, and parses the reply
//! with `httparse`.

use std::{
    io::{Read, Write},
    net::{Ipv4Addr, TcpStream},
    time::Duration,
};

use getset::Getters;
use typed_builder::TypedBuilder;

use super::reply_head::{ReplyHead, get_request};

/// Longest a test waits for one whole reply.
const REPLY_BOUND: Duration = Duration::from_secs(30);

/// One reply the server sent, read until it closed the connection.
#[derive(Clone, Debug, Getters, TypedBuilder)]
pub struct HttpReply {
    /// Its status and headers.
    head: ReplyHead,
    /// The body.
    #[getset(get = "pub")]
    body: Vec<u8>,
}

impl HttpReply {
    /// The status code.
    pub fn status(&self) -> &u16 {
        self.head.status()
    }

    /// The first value of the header `name`, whatever its case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.head.header(name)
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
    let (head, length) = ReplyHead::parse(&bytes).unwrap_or_else(|| {
        panic!(
            "not a whole response: {:?}",
            String::from_utf8_lossy(&bytes)
        )
    });
    HttpReply::builder()
        .head(head)
        .body(bytes[length..].to_vec())
        .build()
}

/// A GET for `target` naming `127.0.0.1:<port>` as its host, with `headers`
/// added, sent to the server on `port`.
///
/// # Panics
/// As [`exchange`].
pub fn get(port: u16, target: &str, headers: &[(&str, &str)]) -> HttpReply {
    exchange(port, get_request(port, target, headers).as_bytes())
}
