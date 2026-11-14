use std::{
    io::{ErrorKind, Read},
    net::TcpStream,
    time::{Duration, Instant},
};

use httparse::{EMPTY_HEADER, Error as ParseError, Status as ParseStatus};

use super::request::Request;

/// Most bytes a request head may take; a longer one is answered 431.
pub const MAX_HEAD_BYTES: usize = 8 * 1024;
/// How long a client has to send its whole head before the connection is
/// closed unanswered.
pub const HEAD_TIMEOUT: Duration = Duration::from_secs(5);
/// Most headers one head may carry; more is answered 431 as an oversized
/// head is.
const MAX_HEADERS: usize = 64;
/// Most bytes one read takes off the socket.
const READ_CHUNK: usize = 1024;

/// How reading one request head ended.
#[derive(Debug)]
pub enum HeadRead {
    /// A complete head.
    Request(Request),
    /// The head ran past [`MAX_HEAD_BYTES`] or [`MAX_HEADERS`].
    TooLarge,
    /// The bytes are not an HTTP request head.
    Malformed,
    /// The client closed, failed or ran out of time first.
    Gone,
}

impl HeadRead {
    /// Reads one request head from `stream` within [`HEAD_TIMEOUT`] of the
    /// call, never holding more than [`MAX_HEAD_BYTES`]. No body is read:
    /// the view serves GET only.
    pub fn from_stream(stream: &mut TcpStream) -> Self {
        let deadline = Instant::now() + HEAD_TIMEOUT;
        let mut head = Vec::with_capacity(READ_CHUNK);
        let mut chunk = [0; READ_CHUNK];
        loop {
            let mut headers = [EMPTY_HEADER; MAX_HEADERS];
            let mut parsed = httparse::Request::new(&mut headers);
            match parsed.parse(&head) {
                Ok(ParseStatus::Complete(_)) => {
                    return Self::Request(Request::from_head(
                        parsed.method.unwrap_or_default(),
                        parsed.path.unwrap_or_default(),
                        parsed.headers,
                    ));
                },
                Ok(ParseStatus::Partial) => {},
                Err(ParseError::TooManyHeaders) => return Self::TooLarge,
                Err(_) => return Self::Malformed,
            }
            if head.len() >= MAX_HEAD_BYTES {
                return Self::TooLarge;
            }
            let Some(left) = deadline
                .checked_duration_since(Instant::now())
                .filter(|left| !left.is_zero())
            else {
                return Self::Gone;
            };
            if stream.set_read_timeout(Some(left)).is_err() {
                return Self::Gone;
            }
            let room = (MAX_HEAD_BYTES - head.len()).min(READ_CHUNK);
            match stream.read(&mut chunk[..room]) {
                Ok(0) => return Self::Gone,
                Ok(read) => head.extend_from_slice(&chunk[..read]),
                Err(error) if error.kind() == ErrorKind::Interrupted => {},
                Err(_) => return Self::Gone,
            }
        }
    }
}
