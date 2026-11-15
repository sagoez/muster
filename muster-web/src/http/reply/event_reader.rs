//! A raw client of a server-sent event stream for tests: it reads the
//! response's head, then the stream a block at a time as the server flushes
//! it. Every read is bounded overall, keepalives included, so a stream
//! that stays open but never sends what a test waits for fails it.

use std::{
    io::{BufRead, BufReader, ErrorKind, Write},
    net::{Ipv4Addr, TcpStream},
    time::{Duration, Instant},
};

use serde_json::Value;

use super::{
    reply_head::{ReplyHead, get_request},
    sse_event::SseEvent,
    sse_frame::{DATA_FIELD, EVENT_FIELD, ID_FIELD},
};

/// Longest a test waits for the head, the next block or event, or an event
/// it looks for, however many keepalives arrive meanwhile.
const STREAM_BOUND: Duration = Duration::from_secs(30);
/// The line ends a line may carry, trimmed off as it is read.
const LINE_ENDS: [char; 2] = ['\r', '\n'];
/// What a test waits for while the head is read.
const HEAD: &str = "the stream's head";
/// What a test waits for when it asks for the next block.
const NEXT_BLOCK: &str = "the next block";
/// What a test waits for when it asks for the next event.
const NEXT_EVENT: &str = "the next event";

/// One open stream: its status and headers, and the rest still to read.
pub struct EventReader {
    /// Its status and headers.
    head: ReplyHead,
    /// The stream after the head.
    reader: BufReader<TcpStream>,
    /// The kind of each event read so far, oldest first, which a timeout
    /// names.
    seen: Vec<String>,
}

impl EventReader {
    /// Sends a GET for `target` naming `127.0.0.1:<port>` as its host to
    /// the server on `port`, and reads the response's head.
    ///
    /// # Panics
    /// When the connection fails, the head is not an HTTP response head, or
    /// it is not whole within [`STREAM_BOUND`].
    pub fn open(port: u16, target: &str) -> Self {
        let deadline = Instant::now() + STREAM_BOUND;
        let mut stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        stream
            .write_all(get_request(port, target, &[]).as_bytes())
            .unwrap();
        let mut reader = BufReader::new(stream);
        let mut raw = Vec::new();
        let head = loop {
            let read = read_line_by(&mut reader, &mut raw, deadline, HEAD, &[]);
            assert!(read > 0, "the stream ended inside its head");
            if let Some((head, _)) = ReplyHead::parse(&raw) {
                break head;
            }
        };
        Self {
            head,
            reader,
            seen: Vec::new(),
        }
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
    /// When the block is not whole within [`STREAM_BOUND`].
    pub fn next_block(&mut self) -> Option<Vec<String>> {
        self.next_block_by(Instant::now() + STREAM_BOUND, NEXT_BLOCK)
    }

    /// The next block naming an event, past comments and a `retry:`;
    /// none once the stream ended.
    ///
    /// # Panics
    /// When no event arrives within [`STREAM_BOUND`], or an event's data is
    /// not JSON.
    pub fn next_event(&mut self) -> Option<SseEvent> {
        self.next_event_by(Instant::now() + STREAM_BOUND, NEXT_EVENT)
    }

    /// The first event from here on for which `wanted` holds, reading past
    /// the others.
    ///
    /// # Panics
    /// When the stream ends or no such event arrives within
    /// [`STREAM_BOUND`]; `what` names it.
    #[cfg(feature = "testing")]
    pub fn event_where(
        &mut self,
        what: &str,
        mut wanted: impl FnMut(&SseEvent) -> bool,
    ) -> SseEvent {
        let deadline = Instant::now() + STREAM_BOUND;
        loop {
            let event = self
                .next_event_by(deadline, what)
                .unwrap_or_else(|| panic!("the stream ended before {what}"));
            if wanted(&event) {
                return event;
            }
        }
    }

    /// The next event, as [`Self::next_event`], failing past `deadline`
    /// with a message naming `what` it waited for.
    fn next_event_by(&mut self, deadline: Instant, what: &str) -> Option<SseEvent> {
        loop {
            let block = self.next_block_by(deadline, what)?;
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
            self.seen.push(kind.clone());
            return Some(
                SseEvent::builder()
                    .kind(kind)
                    .id(field(ID_FIELD))
                    .data(data)
                    .build(),
            );
        }
    }

    /// The next block, as [`Self::next_block`], failing past `deadline`
    /// with a message naming `what` it waited for.
    fn next_block_by(&mut self, deadline: Instant, what: &str) -> Option<Vec<String>> {
        let mut block = Vec::new();
        loop {
            let mut raw = Vec::new();
            if read_line_by(&mut self.reader, &mut raw, deadline, what, &self.seen) == 0 {
                return None;
            }
            let line = String::from_utf8_lossy(&raw)
                .trim_end_matches(LINE_ENDS)
                .to_string();
            if line.is_empty() {
                return Some(block);
            }
            block.push(line);
        }
    }
}

/// Reads one line of `reader` onto `raw`, returning how many bytes it
/// added: none once the stream ended.
///
/// # Panics
/// When the read fails, or the line is not whole by `deadline`; the
/// message names `what` it waited for and the kinds of the events `seen`.
fn read_line_by(
    reader: &mut BufReader<TcpStream>,
    raw: &mut Vec<u8>,
    deadline: Instant,
    what: &str,
    seen: &[String],
) -> usize {
    let start = raw.len();
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        assert!(
            !left.is_zero(),
            "timed out after {STREAM_BOUND:?} waiting for {what}; events seen: {seen:?}"
        );
        reader.get_ref().set_read_timeout(Some(left)).unwrap();
        match reader.read_until(b'\n', raw) {
            Ok(0) => return raw.len() - start,
            Ok(_) if raw.ends_with(b"\n") => return raw.len() - start,
            Ok(_) => {},
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                ) => {},
            Err(error) => panic!("reading the stream for {what} failed: {error}"),
        }
    }
}
