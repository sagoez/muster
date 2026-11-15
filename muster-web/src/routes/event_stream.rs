use std::{io, time::Duration};

use crossbeam_channel::RecvTimeoutError;

use crate::{
    WebError,
    floor::Sequenced,
    http::{Response, StreamOut, Streamed},
    live::Subscription,
};

/// How long a page waits before reconnecting a stream that dropped, in
/// milliseconds.
const RETRY_MILLIS: u64 = 1_000;
/// How long the stream may stay silent before a comment keeps it open. Each
/// wake also looks whether the page hung up, so a closed tab gives its
/// connection back within this long: a browser opens at most six to one
/// host, and every tab's stream holds one.
pub const KEEPALIVE_AFTER: Duration = Duration::from_secs(5);
/// The field telling a page how long to wait before reconnecting.
const RETRY_FIELD: &str = "retry: ";
/// The field that names a frame's event.
const EVENT_FIELD: &str = "event: ";
/// The field that carries a frame's id, which a reconnecting browser sends
/// back.
const ID_FIELD: &str = "id: ";
/// The field that carries a frame's data.
const DATA_FIELD: &str = "data: ";
/// What ends one line of a block.
const LINE_END: &str = "\n";
/// What ends a block of the stream.
const BLOCK_END: &str = "\n\n";
/// The comment that keeps a silent stream open; a page ignores it.
const KEEPALIVE: &str = ": keepalive";

/// `message` as one Server-Sent Events frame: its kind as the event, its
/// sequence number as the id, and its one line of JSON as the data.
///
/// # Errors
/// Returns [`WebError::Frame`] when the message cannot be written as JSON.
pub fn frame(message: &Sequenced) -> Result<String, WebError> {
    let kind: &'static str = message.message().into();
    Ok(block(&format!(
        "{EVENT_FIELD}{kind}{LINE_END}{ID_FIELD}{}{LINE_END}{DATA_FIELD}{}",
        message.seq(),
        message.json()?
    )))
}

/// `lines` ended as one block of the stream.
fn block(lines: &str) -> String {
    format!("{lines}{BLOCK_END}")
}

/// The live stream one page reads: its own close-delimited response,
/// telling the page to reconnect after a second, then every frame the hub
/// sends, each flushed as it comes, until the hub drops the stream, the page
/// stops reading or it hangs up.
pub struct EventStream {
    /// The hub's frames for this page, its snapshot first.
    subscription: Subscription,
}

impl EventStream {
    /// The stream of `subscription`'s frames.
    pub fn new(subscription: Subscription) -> Self {
        Self { subscription }
    }
}

impl Streamed for EventStream {
    /// Writes the head and the reconnect delay, then each frame, and a
    /// comment after [`KEEPALIVE_AFTER`] of silence; returns once the hub
    /// drops the stream, a write fails, or the page is seen to have hung up
    /// before a write.
    fn stream(self: Box<Self>, out: &mut dyn StreamOut) {
        let retry = block(&format!("{RETRY_FIELD}{RETRY_MILLIS}"));
        if Response::event_stream().write_to(out).is_err() || send(out, &retry).is_err() {
            return;
        }
        let keepalive = block(KEEPALIVE);
        loop {
            let next = self.subscription.frames().recv_timeout(KEEPALIVE_AFTER);
            if matches!(next, Err(RecvTimeoutError::Disconnected)) || out.hung_up() {
                return;
            }
            let sent = match next {
                Ok(frame) => send(out, &frame),
                Err(_) => send(out, &keepalive),
            };
            if sent.is_err() {
                return;
            }
        }
    }
}

/// Writes `text` to `out` and flushes it, so the page has it at once.
///
/// # Errors
/// Returns the failure of the write.
fn send(out: &mut dyn StreamOut, text: &str) -> io::Result<()> {
    out.write_all(text.as_bytes())?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use std::{io::Write, sync::mpsc, thread};

    use muster_domain::value::{ProcessName, ProjectName};
    use serde_json::Value;

    use super::*;
    use crate::{
        floor::{FloorMessage, fixture::floor},
        live::Hub,
    };

    /// Longest a test waits for a stream to end.
    const BOUND: Duration = Duration::from_secs(10);

    /// A client that has hung up and takes nothing written to it.
    struct Gone;

    impl Write for Gone {
        fn write(&mut self, written: &[u8]) -> io::Result<usize> {
            Ok(written.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl StreamOut for Gone {
        fn hung_up(&mut self) -> bool {
            true
        }
    }

    /// The stream is a close-delimited event stream that starts by telling
    /// the page to reconnect after a second, sends the snapshot, and ends
    /// once the hub does.
    #[test]
    fn the_stream_starts_with_retry_and_ends_with_the_hub() {
        let (hub, handle) = Hub::new(floor());
        let thread = hub.spawn().unwrap();
        let subscription = handle.subscribe().unwrap().unwrap();
        handle.stop();
        thread.join().unwrap();
        let mut written = Vec::new();

        Box::new(EventStream::new(subscription)).stream(&mut written);

        let text = String::from_utf8(written).unwrap();
        let (head, body) = text.split_once("\r\n\r\n").unwrap();
        assert!(head.contains("Content-Type: text/event-stream"), "{head}");
        assert!(!head.contains("Content-Length"), "{head}");
        assert!(
            body.starts_with("retry: 1000\n\nevent: snapshot\nid: "),
            "{body}"
        );
        assert!(body.ends_with(BLOCK_END), "{body}");
    }

    /// A page that hung up ends its stream at the next wake, though the hub
    /// still runs and would keep it open.
    #[test]
    fn a_page_that_hung_up_ends_its_stream() {
        let (hub, handle) = Hub::new(floor());
        let _thread = hub.spawn().unwrap();
        let subscription = handle.subscribe().unwrap().unwrap();
        let (ended, done) = mpsc::channel();

        thread::spawn(move || {
            Box::new(EventStream::new(subscription)).stream(&mut Gone);
            let _ = ended.send(());
        });

        assert!(done.recv_timeout(BOUND).is_ok());
        handle.stop();
    }

    /// A frame is one event naming the message's kind, its sequence number
    /// as the id, and the message's JSON as one line of data.
    #[test]
    fn a_frame_is_one_event_with_one_line_of_data() {
        let message = FloorMessage::Speaking {
            project: ProjectName::try_new("shop").unwrap(),
            name: ProcessName::try_new("web").unwrap(),
            on: true,
        };

        let frame = frame(&Sequenced::builder().seq(7).message(message).build()).unwrap();

        let lines: Vec<&str> = frame
            .strip_suffix(BLOCK_END)
            .unwrap()
            .split(LINE_END)
            .collect();
        assert_eq!(lines.len(), 3, "{frame}");
        assert_eq!(lines[0], "event: speaking");
        assert_eq!(lines[1], "id: 7");
        let data: Value = serde_json::from_str(lines[2].strip_prefix(DATA_FIELD).unwrap()).unwrap();
        assert_eq!(data["kind"], "speaking");
        assert_eq!(data["seq"], 7);
        assert_eq!(data["name"], "web");
    }
}
