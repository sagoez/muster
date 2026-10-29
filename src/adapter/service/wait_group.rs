use crossbeam_channel::Sender;

use super::{
    batch_outcome::BatchOutcome, service_response::ServiceResponse, wait_outcome::WaitOutcome,
};

/// The waits of one project-wide start, answered together once each ended.
pub struct WaitGroup {
    /// What the start did.
    batch: BatchOutcome,
    /// How each wait ended, by slot, once it did.
    outcomes: Vec<Option<WaitOutcome>>,
    /// Where the answer goes.
    reply: Sender<ServiceResponse>,
}

impl WaitGroup {
    /// A group of `slots` waits on what `batch` started, answered on `reply`.
    pub fn new(batch: BatchOutcome, slots: usize, reply: Sender<ServiceResponse>) -> Self {
        Self {
            batch,
            outcomes: vec![None; slots],
            reply,
        }
    }

    /// Fills `slot` with `outcome`. Once every slot is filled, answers the
    /// batch and returns `true`.
    pub fn fill(&mut self, slot: usize, outcome: WaitOutcome) -> bool {
        if let Some(entry) = self.outcomes.get_mut(slot) {
            *entry = Some(outcome);
        }
        if self.outcomes.iter().any(Option::is_none) {
            return false;
        }
        let waits = self.outcomes.iter_mut().filter_map(Option::take).collect();
        let _ = self
            .reply
            .send(ServiceResponse::Batch(self.batch.clone().with_waits(waits)));
        true
    }
}
