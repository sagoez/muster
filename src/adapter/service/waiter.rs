use std::time::Instant;

use crossbeam_channel::Sender;
use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use super::{
    batch_outcome::BatchOutcome, service_response::ServiceResponse, supervisor::SpawnGeneration,
    wait_condition::WaitCondition, wait_outcome::WaitOutcome,
};
use crate::{
    adapter::holder::LineMatcher,
    domain::value::{OutputCursor, PaneId, ProcessName},
};

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

/// Which runs of a process a wait is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunPin {
    /// The run current at `from` and every later one.
    From(SpawnGeneration),
    /// Only runs started after `after`, so a start or restart never sees the
    /// run it replaced.
    After(SpawnGeneration),
}

impl RunPin {
    /// Whether the run `generation` is one the wait is about.
    pub fn counts(self, generation: SpawnGeneration) -> bool {
        match self {
            Self::From(from) => generation >= from,
            Self::After(after) => generation > after,
        }
    }

    /// Whether the wait is only about a run its own request starts.
    pub fn is_fresh(self) -> bool {
        matches!(self, Self::After(_))
    }
}

/// A wait registered on the service loop, answered when its condition holds,
/// its run ends, or its deadline passes.
#[derive(Getters, CopyGetters, TypedBuilder)]
pub struct Waiter {
    /// The pane of the process waited on.
    #[getset(get_copy = "pub")]
    pane: PaneId,
    /// The process waited on.
    #[getset(get = "pub")]
    process: ProcessName,
    /// What it waits for, the default resolved.
    #[getset(get = "pub")]
    until: WaitCondition,
    /// The compiled pattern of a log condition.
    #[getset(get = "pub")]
    #[builder(default)]
    matcher: Option<LineMatcher>,
    /// The runs it is about.
    #[getset(get_copy = "pub")]
    pin: RunPin,
    /// When it was registered.
    #[getset(get_copy = "pub")]
    started: Instant,
    /// When it answers that it is still waiting.
    #[getset(get_copy = "pub")]
    deadline: Instant,
    /// Where its answer goes.
    #[getset(get = "pub")]
    reply: WaitReply,
    /// Where scanning began, when the asked cursor was older than the output
    /// held.
    #[getset(get_copy = "pub")]
    #[builder(default)]
    scanned_from: Option<OutputCursor>,
}

impl Waiter {
    /// Takes its reply, to answer it.
    pub fn into_reply(self) -> WaitReply {
        self.reply
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_pin_skips_the_run_it_replaced() {
        let current = SpawnGeneration::new(4);

        assert!(!RunPin::After(current).counts(current));
        assert!(RunPin::After(current).counts(current.next()));
        assert!(RunPin::From(current).counts(current));
        assert!(!RunPin::From(current).counts(SpawnGeneration::new(3)));
    }
}
