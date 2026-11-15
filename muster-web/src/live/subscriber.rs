use std::sync::Arc;

use crossbeam_channel::Sender;

/// One open stream, as the hub holds it: a private helper of
/// [`Subscribers`](super::subscribers::Subscribers), which builds each with a
/// struct literal.
pub(super) struct Subscriber {
    /// Where its frames go.
    pub(super) frames: Sender<Arc<str>>,
    /// Shared with its [`Subscription`](super::subscription::Subscription),
    /// so a count above one means it is still open.
    pub(super) presence: Arc<()>,
}
