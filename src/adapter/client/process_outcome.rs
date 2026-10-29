use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::{outcome::Outcome, process_view::ProcessView};
use crate::domain::value::ProcessName;

/// One process a command acted on: what became of it and how it is listed
/// now.
#[derive(Debug, Clone, PartialEq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct ProcessOutcome {
    /// The process.
    #[getset(get = "pub")]
    name: ProcessName,
    /// What became of it.
    #[getset(get_copy = "pub")]
    outcome: Outcome,
    /// Why the service left it alone, for a `refused` outcome.
    #[builder(default)]
    #[getset(get = "pub")]
    reason: Option<String>,
    /// How `ps` lists it now, unless it is no longer listed.
    #[getset(get = "pub")]
    status: Option<ProcessView>,
}
