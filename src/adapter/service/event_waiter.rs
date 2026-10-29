use std::time::Instant;

use crossbeam_channel::Sender;

use super::{event_kind::EventKind, event_seq::EventSeq, service_response::ServiceResponse};
use crate::domain::value::ProjectRoot;

/// A client waiting for a project's next lifecycle event.
pub(super) struct EventWaiter {
    /// The project whose events it reads.
    pub(super) project: ProjectRoot,
    /// Events after this one count.
    pub(super) since: EventSeq,
    /// The kinds it reads, every kind when empty.
    pub(super) kinds: Vec<EventKind>,
    /// When it is answered with no events.
    pub(super) deadline: Instant,
    /// Where the answer goes.
    pub(super) reply: Sender<ServiceResponse>,
}
