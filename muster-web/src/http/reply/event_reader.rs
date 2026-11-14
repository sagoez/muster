//! A raw client of a server-sent event stream for tests: it reads the
//! response's head, then the stream a block at a time as the server flushes
//! it.

use std::{
    io::{BufRead, BufReader, Write},
    net::{Ipv4Addr, TcpStream},
    time::{Duration, Instant},
};

use getset::Getters;
use serde_json::Value;

use super::sse_event::SseEvent;

/// Longest a test waits for one line of the stream.
const LINE_BOUND: Duration = Duration::from_secs(30);
/// Longest a test waits for an event it looks for.
const EVENT_BOUND: Duration = Duration::from_secs(30);
/// The field naming a block's event.
const EVENT_FIELD: &str = "event: ";
/// The field carrying a block's id.
const ID_FIELD: &str = "id: ";
/// The field carrying a block's data.
const DATA_FIELD: &str = "data: ";
/// What separates a header's name from its value.
const HEADER_SEPARATOR: &str = ": ";

/// One open stream: its status and headers, and the rest still to read.
#[derive(Getters)]
pub struct EventReader {
    /// The status code.
    #[getset(get = "pub")]
    status: u16,
    /// Every header, in order, with its name as sent.
    #[getset(get = "pub")]
    headers: Vec<(String, String)>,
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
            .write_all(
                format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\r\n").as_bytes(),
            )
            .unwrap();
        let mut reader = BufReader::new(stream);
        let status_line = read_line(&mut reader).expect("no status line");
        let status = status_line
            .split(' ')
            .nth(1)
            .and_then(|code| code.parse().ok())
            .unwrap_or_else(|| panic!("not a status line: {status_line:?}"));
        let mut headers = Vec::new();
        while let Some(line) = read_line(&mut reader).filter(|line| !line.is_empty()) {
            let (name, value) = line
                .split_once(HEADER_SEPARATOR)
                .unwrap_or_else(|| panic!("not a header: {line:?}"));
            headers.push((name.to_string(), value.to_string()));
        }
        Self {
            status,
            headers,
            reader,
        }
    }

    /// The first value of the header `name`, whatever its case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(sent, _)| sent.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
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
    Some(line.trim_end_matches(['\r', '\n']).to_string())
}
