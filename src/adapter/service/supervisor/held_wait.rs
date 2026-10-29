use crossbeam_channel::Sender;

use super::{spawn_generation::SpawnGeneration, waiting::PreparedWait};
use crate::adapter::service::ServiceResponse;

/// A wait on a process this service never saw run, held while its log's end
/// is read off the loop, so it is answered with the log's cursor and tail.
pub(super) struct HeldWait {
    /// Which held wait this is, so its timeout refuses it alone.
    pub(super) id: u64,
    /// The wait, checked against its process.
    pub(super) prepared: PreparedWait,
    /// Only runs started after this generation count, when given.
    pub(super) after: Option<SpawnGeneration>,
    /// Where the answer goes.
    pub(super) reply: Sender<ServiceResponse>,
}
