use std::time::Instant;

use crate::{
    adapter::service::{WaitOutcome, WaitReply},
    domain::value::{OutputCursor, PaneId},
};

/// The answer of a wait that did not match, held while its run's last lines
/// are read from the log because the tap holds too few of them, as after an
/// adoption.
pub(super) struct HeldAnswer {
    /// The pane of the process.
    pub(super) pane: PaneId,
    /// The key its read's answer comes back under.
    pub(super) key: u64,
    /// The answer, with the lines the tap holds as its tail.
    pub(super) outcome: WaitOutcome,
    /// Where it goes.
    pub(super) reply: WaitReply,
    /// Where the run began.
    pub(super) from: OutputCursor,
    /// Where its output stood.
    pub(super) until: OutputCursor,
    /// When it is sent as it is if the read has not landed.
    pub(super) deadline: Instant,
    /// The read asked for its lines, once one was.
    pub(super) read: Option<u64>,
}
