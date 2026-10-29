use crossbeam_channel::Sender;

use super::service_response::ServiceResponse;

/// Where the answer to one wait goes.
pub enum WaitReply {
    /// Straight to the client that asked.
    Direct(Sender<ServiceResponse>),
    /// Into one slot of a batch of waits answered together.
    Batch {
        /// The batch.
        group: u64,
        /// The slot.
        slot: usize,
    },
}
