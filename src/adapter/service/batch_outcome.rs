use getset::Getters;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::{refusal::Refusal, wait_outcome::WaitOutcome};
use crate::domain::value::ProcessName;

/// What `Up` or `Down` did to a project: the processes it started or
/// stopped, those it left alone with why, and how each wait it was asked for
/// ended.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
#[serde(rename_all = "kebab-case")]
pub struct BatchOutcome {
    /// The processes it started or stopped, in config section order.
    acted: Vec<ProcessName>,
    /// The processes it would have acted on but could not.
    refused: Vec<Refusal>,
    /// The processes `Up` found running or starting already, in config
    /// section order.
    #[builder(default)]
    already_running: Vec<ProcessName>,
    /// How the wait on each process it started ended, in the order of
    /// `acted`, when it was asked to wait.
    #[builder(default)]
    waits: Vec<WaitOutcome>,
}

impl BatchOutcome {
    /// This outcome with `already_running`.
    #[must_use]
    pub fn with_already_running(self, already_running: Vec<ProcessName>) -> Self {
        Self {
            already_running,
            ..self
        }
    }

    /// Whether it acted on nothing and refused nothing.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.acted.is_empty() && self.refused.is_empty()
    }

    /// This outcome with `waits`.
    #[must_use]
    pub fn with_waits(self, waits: Vec<WaitOutcome>) -> Self {
        Self { waits, ..self }
    }
}
