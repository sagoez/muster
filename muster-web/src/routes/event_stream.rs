use std::{io, time::Duration};

use crossbeam_channel::RecvTimeoutError;

use crate::{
    http::{Response, StreamOut, Streamed, comment_frame, retry_frame},
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
/// The comment that keeps a silent stream open; a page ignores it.
const KEEPALIVE: &str = "keepalive";

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
        if Response::event_stream().write_to(out).is_err()
            || send(out, &retry_frame(RETRY_MILLIS)).is_err()
        {
            return;
        }
        let keepalive = comment_frame(KEEPALIVE);
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
    use std::{sync::mpsc, thread};

    use super::*;
    use crate::{floor::fixture::floor, http::BLOCK_END, live::Hub, routes::gone::Gone};

    /// Longest a test waits for a stream to end.
    const BOUND: Duration = Duration::from_secs(10);

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
}
