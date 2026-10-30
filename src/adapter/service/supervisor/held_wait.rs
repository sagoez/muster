use std::time::Instant;

use crossbeam_channel::Sender;

use super::{held_scan::HeldScan, spawn_generation::SpawnGeneration, waiting::PreparedWait};
use crate::{adapter::service::ServiceResponse, domain::value::OutputCursor};

/// A wait held while its process's log is read off the loop: the log's end
/// for a process this service never saw run, so it is answered with the
/// log's cursor and tail, or the slice from its cursor up to what the tap
/// holds, so every line from that cursor is tested. A wait from a cursor
/// asked while its process is being adopted is held until the adoption
/// settles, then held on that slice like any other; one whose own timeout
/// passes first, before a tap holds its run, is held on the log's end only
/// for its answer's tail.
pub(super) struct HeldWait {
    /// The wait, checked against its process.
    pub(super) prepared: PreparedWait,
    /// Only runs started after this generation count, when given.
    pub(super) after: Option<SpawnGeneration>,
    /// Where the answer goes.
    pub(super) reply: Sender<ServiceResponse>,
    /// When it is refused or answered if the read has not let it go, or
    /// times out if the adoption has not.
    pub(super) deadline: Instant,
    /// The key its scan's answer comes back under.
    pub(super) key: u64,
    /// The scan it is held on; none while it waits on the log's end or the
    /// adoption.
    pub(super) scan: Option<HeldScan>,
    /// The cursor it waits from while it waits for its process's adoption
    /// to settle, reading nothing meanwhile.
    pub(super) adopting: Option<OutputCursor>,
    /// The cursor it timed out at, its own, once its timeout passed while
    /// it was held on an adoption: the log's end is then read only for the
    /// tail its timed-out answer carries.
    pub(super) timed_out: Option<OutputCursor>,
    /// The read asked for its scan, once one was.
    pub(super) read: Option<u64>,
}
