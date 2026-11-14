use std::sync::Arc;

use crossbeam_channel::{Sender, bounded};

use super::subscription::Subscription;
use crate::floor::Sequenced;

/// Most pages streaming at once; one more is refused.
pub const MAX_STREAMS: usize = 16;
/// Most frames a stream holds unread; a page that falls this far behind is
/// dropped and catches up with a snapshot when it reconnects.
pub const STREAM_BUFFER: usize = 256;

/// One open stream, as the hub holds it.
struct Subscriber {
    /// Where its frames go.
    frames: Sender<Arc<str>>,
    /// Shared with its [`Subscription`], so a count above one means it is
    /// still open.
    presence: Arc<()>,
}

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
        self.open
            .retain(|subscriber| Arc::strong_count(&subscriber.presence) > 1);
        self.open.len() >= MAX_STREAMS
    }

    /// A new stream whose first frame is `snapshot`, sent ahead of every
    /// later broadcast; none when the snapshot cannot be written.
    pub fn admit(&mut self, snapshot: &Sequenced) -> Option<Subscription> {
        let frame: Arc<str> = snapshot.frame().ok()?.into();
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
            let Ok(frame) = message.frame() else {
                continue;
            };
            let frame: Arc<str> = frame.into();
            self.open
                .retain(|subscriber| subscriber.frames.try_send(Arc::clone(&frame)).is_ok());
        }
    }
}
