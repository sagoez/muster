use std::sync::Arc;

use crossbeam_channel::Receiver;
use getset::Getters;

/// One page's stream of frames from the hub, its snapshot first. Dropping
/// it frees its place among the streams.
#[derive(Debug, Getters)]
pub struct Subscription {
    /// Each frame, in the order the hub sent them; disconnected once the
    /// hub drops the stream or ends.
    #[getset(get = "pub")]
    frames: Receiver<Arc<str>>,
    /// Held while the stream is open, so the hub can count the open ones.
    _presence: Arc<()>,
}

impl Subscription {
    /// The stream reading `frames`, holding `presence` while it is open.
    pub(super) fn new(frames: Receiver<Arc<str>>, presence: Arc<()>) -> Self {
        Self {
            frames,
            _presence: presence,
        }
    }
}
