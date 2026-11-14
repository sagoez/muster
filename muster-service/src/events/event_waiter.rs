use std::time::Instant;

use crossbeam_channel::Sender;
use muster_domain::value::ProjectRoot;
use muster_protocol::service::{EventKind, EventSeq, ServiceResponse};

/// A client waiting for a project's next lifecycle event.
pub(crate) struct EventWaiter {
    /// The project whose events it reads.
    pub(crate) project: ProjectRoot,
    /// Events after this one count.
    pub(crate) since: EventSeq,
    /// The kinds it reads, every kind when empty.
    pub(crate) kinds: Vec<EventKind>,
    /// When it is answered with no events.
    pub(crate) deadline: Instant,
    /// Where the answer goes.
    pub(crate) reply: Sender<ServiceResponse>,
}
