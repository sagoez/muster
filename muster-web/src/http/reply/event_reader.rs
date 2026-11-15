//! A raw client of a server-sent event stream for tests: it reads the
//! response's head, then the stream a block at a time as the server flushes
//! it.

#[cfg(feature = "testing")]
use std::time::Instant;
use std::{
    io::{BufRead, BufReader, Write},
    net::{Ipv4Addr, TcpStream},
    time::Duration,
};

use serde_json::Value;

use super::{
    reply_head::{ReplyHead, get_request},
    sse_event::SseEvent,
};

/// Longest a test waits for one line of the stream.
const LINE_BOUND: Duration = Duration::from_secs(30);
/// Longest a test waits for an event it looks for.
#[cfg(feature = "testing")]
const EVENT_BOUND: Duration = Duration::from_secs(30);
/// The field naming a block's event.
const EVENT_FIELD: &str = "event: ";
/// The field carrying a block's id.
const ID_FIELD: &str = "id: ";
/// The field carrying a block's data.
const DATA_FIELD: &str = "data: ";
/// The line ends a line may carry, trimmed off as it is read.
const LINE_ENDS: [char; 2] = ['\r', '\n'];

/// One open stream: its status and headers, and the rest still to read.
pub struct EventReader {
    /// Its status and headers.
    head: ReplyHead,
    /// The stream after the head.
    reader: BufReader<TcpStream>,
}

impl EventReader {
    /// Sends a GET for `target` naming `127.0.0.1:<port>` as its host to
    /// the server on `port`, and reads the response's head.
    ///
    /// # Panics
    /// When the connection fails or the head is not an HTTP response head.
    pub fn open(port: u16, target: &str) -> Self {
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        stream.set_read_timeout(Some(LINE_BOUND)).unwrap();
        stream
            .write_all(get_request(port, target, &[]).as_bytes())
            .unwrap();
        let mut reader = BufReader::new(stream);
        let mut raw = Vec::new();
        let head = loop {
            let read = reader.read_until(b'\n', &mut raw).unwrap();
            assert!(read > 0, "the stream ended inside its head");
            if let Some((head, _)) = ReplyHead::parse(&raw) {
                break head;
            }
        };
        Self { head, reader }
    }

    /// The status code.
    pub fn status(&self) -> &u16 {
        self.head.status()
    }

    /// The first value of the header `name`, whatever its case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.head.header(name)
    }

    /// The next block of lines up to the blank line that ends it, each
    /// without its line end; none once the stream ended.
    ///
    /// # Panics
    /// When no line arrives in time.
    pub fn next_block(&mut self) -> Option<Vec<String>> {
        let mut block = Vec::new();
        loop {
            let line = read_line(&mut self.reader)?;
            if line.is_empty() {
                return Some(block);
            }
            block.push(line);
        }
    }

    /// The next block naming an event, past comments and a `retry:`;
    /// none once the stream ended.
    ///
    /// # Panics
    /// When no line arrives in time, or an event's data is not JSON.
    pub fn next_event(&mut self) -> Option<SseEvent> {
        loop {
            let block = self.next_block()?;
            let field = |name: &str| {
                block
                    .iter()
                    .find_map(|line| line.strip_prefix(name))
                    .map(str::to_string)
            };
            let Some(kind) = field(EVENT_FIELD) else {
                continue;
            };
            let data: Value = serde_json::from_str(&field(DATA_FIELD).unwrap_or_default())
                .unwrap_or_else(|error| panic!("{kind} data is not JSON: {error}"));
            return Some(
                SseEvent::builder()
                    .kind(kind)
                    .id(field(ID_FIELD))
                    .data(data)
                    .build(),
            );
        }
    }

    /// The first event from here on for which `wanted` holds, reading past
    /// the others.
    ///
    /// # Panics
    /// When the stream ends or no such event arrives in time; `what` names
    /// it.
    #[cfg(feature = "testing")]
    pub fn event_where(
        &mut self,
        what: &str,
        mut wanted: impl FnMut(&SseEvent) -> bool,
    ) -> SseEvent {
        let started = Instant::now();
        loop {
            assert!(
                started.elapsed() < EVENT_BOUND,
                "timed out waiting for {what}"
            );
            let event = self
                .next_event()
                .unwrap_or_else(|| panic!("the stream ended before {what}"));
            if wanted(&event) {
                return event;
            }
        }
    }
}

/// The next line of `reader` without its line end; none once it ended.
///
/// # Panics
/// When the read fails or times out.
fn read_line(reader: &mut BufReader<TcpStream>) -> Option<String> {
    let mut line = String::new();
    let read = reader.read_line(&mut line).unwrap();
    if read == 0 {
        return None;
    }
    Some(line.trim_end_matches(LINE_ENDS).to_string())
}
