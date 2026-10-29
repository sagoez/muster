use getset::Getters;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::refusal::Refusal;
use crate::domain::value::ProcessName;

/// What `Up` or `Down` did to a project: the processes it started or
/// stopped, and those it left alone with why.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
#[serde(rename_all = "kebab-case")]
pub struct BatchOutcome {
    /// The processes it started or stopped, in config section order.
    acted: Vec<ProcessName>,
    /// The processes it would have acted on but could not.
    refused: Vec<Refusal>,
}

impl BatchOutcome {
    /// Whether it acted on nothing and refused nothing.
    pub fn is_empty(&self) -> bool {
        self.acted.is_empty() && self.refused.is_empty()
    }
}
