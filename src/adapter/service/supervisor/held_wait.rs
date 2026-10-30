use std::time::Instant;

use crossbeam_channel::Sender;

use super::{held_scan::HeldScan, spawn_generation::SpawnGeneration, waiting::PreparedWait};
use crate::adapter::service::ServiceResponse;

/// A wait held while its process's log is read off the loop: the log's end
/// for a process this service never saw run, so it is answered with the
/// log's cursor and tail, or the slice from its cursor up to what the tap
/// holds, so every line from that cursor is tested.
pub(super) struct HeldWait {
    /// The wait, checked against its process.
    pub(super) prepared: PreparedWait,
    /// Only runs started after this generation count, when given.
    pub(super) after: Option<SpawnGeneration>,
    /// Where the answer goes.
    pub(super) reply: Sender<ServiceResponse>,
    /// When it was held.
    pub(super) held_at: Instant,
    /// When it is refused if the read has not answered it.
    pub(super) deadline: Instant,
    /// The key its scan's answer comes back under.
    pub(super) key: u64,
    /// The scan it is held on; none while it waits on the log's end.
    pub(super) scan: Option<HeldScan>,
    /// The read asked for its scan, once one was.
    pub(super) read: Option<u64>,
}
