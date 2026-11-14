use std::time::Instant;

use muster_domain::value::{OutputCursor, PaneId};
use muster_protocol::service::WaitOutcome;

use crate::WaitReply;

/// The answer of a wait that did not match, held while its run's last lines
/// are read from the log because the tap holds too few of them, as after an
/// adoption.
pub(crate) struct HeldAnswer {
    /// The pane of the process.
    pub(crate) pane: PaneId,
    /// The key its read's answer comes back under.
    pub(crate) key: u64,
    /// The answer, with the lines the tap holds as its tail.
    pub(crate) outcome: WaitOutcome,
    /// Where it goes.
    pub(crate) reply: WaitReply,
    /// Where the run began.
    pub(crate) from: OutputCursor,
    /// Where its output stood.
    pub(crate) until: OutputCursor,
    /// When it is sent as it is if the read has not landed.
    pub(crate) deadline: Instant,
    /// The read asked for its lines, once one was.
    pub(crate) read: Option<u64>,
}
