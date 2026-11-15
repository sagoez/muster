use std::sync::Arc;

use crossbeam_channel::bounded;

use super::{subscriber::Subscriber, subscription::Subscription};
use crate::{WebError, floor::Sequenced, http::event_frame};

/// Most pages streaming at once; one more is refused.
pub const MAX_STREAMS: usize = 16;
/// Most frames a stream holds unread; a page that falls this far behind is
/// dropped and catches up with a snapshot when it reconnects.
pub const STREAM_BUFFER: usize = 256;

/// Every open stream, which the hub sends each message to without ever
/// waiting on one.
#[derive(Default)]
pub struct Subscribers {
    /// The open streams, in the order they came.
    open: Vec<Subscriber>,
}

impl Subscribers {
    /// Whether [`MAX_STREAMS`] streams are open, counting none that closed.
    pub fn full(&mut self) -> bool {
        self.forget_closed();
        self.open.len() >= MAX_STREAMS
    }

    /// Whether any stream is open, counting none that closed.
    pub fn any_open(&mut self) -> bool {
        self.forget_closed();
        !self.open.is_empty()
    }

    /// Drops every stream whose page closed it.
    fn forget_closed(&mut self) {
        self.open
            .retain(|subscriber| Arc::strong_count(&subscriber.presence) > 1);
    }

    /// A new stream whose first frame is `snapshot`, sent ahead of every
    /// later broadcast; none when the snapshot cannot be written.
    pub fn admit(&mut self, snapshot: &Sequenced) -> Option<Subscription> {
        let frame: Arc<str> = frame(snapshot).ok()?.into();
        let (sender, frames) = bounded(STREAM_BUFFER);
        sender.try_send(frame).ok()?;
        let presence = Arc::new(());
        self.open.push(Subscriber {
            frames: sender,
            presence: Arc::clone(&presence),
        });
        Some(Subscription::new(frames, presence))
    }

    /// Sends each of `messages` to every open stream, without waiting: a
    /// stream that is full or closed is dropped.
    pub fn broadcast(&mut self, messages: &[Sequenced]) {
        for message in messages {
            let Ok(frame) = frame(message) else {
                continue;
            };
            let frame: Arc<str> = frame.into();
            self.open
                .retain(|subscriber| subscriber.frames.try_send(Arc::clone(&frame)).is_ok());
        }
    }
}

/// `message` as one Server-Sent Events frame: its kind as the event, its
/// sequence number as the id, and its one line of JSON as the data.
///
/// # Errors
/// Returns [`WebError::Frame`] when the message cannot be written as JSON.
fn frame(message: &Sequenced) -> Result<String, WebError> {
    let kind: &'static str = message.message().into();
    Ok(event_frame(kind, message.seq(), &message.json()?))
}

#[cfg(test)]
mod tests {
    use muster_domain::value::{ProcessName, ProjectName};
    use serde_json::Value;

    use super::*;
    use crate::{
        floor::FloorMessage,
        http::{BLOCK_END, DATA_FIELD, LINE_END},
    };

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
