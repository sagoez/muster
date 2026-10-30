use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::{outcome::Outcome, process_outcome::ProcessOutcome};
use crate::domain::value::ProcessName;

/// One process removed from a stack, by `muster rm`, `muster import
/// --replace` or `process_remove`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct RemovedView {
    /// The process, as the stack spelt it.
    #[getset(get = "pub")]
    name: ProcessName,
    /// Whether it was running and was stopped.
    #[getset(get_copy = "pub")]
    stopped: bool,
    /// Whether it was still stopping when the wait ended; it still ends
    /// through its stop policy.
    #[getset(get_copy = "pub")]
    still_stopping: bool,
}

impl From<&ProcessOutcome> for RemovedView {
    fn from(process: &ProcessOutcome) -> Self {
        Self {
            name: process.name().clone(),
            stopped: process.outcome() == Outcome::Stopped,
            still_stopping: matches!(
                process.outcome(),
                Outcome::StillStopping | Outcome::StillRunning
            ),
        }
    }
}
