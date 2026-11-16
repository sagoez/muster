use std::time::SystemTime;

use getset::Getters;
use typed_builder::TypedBuilder;

use crate::value::ProcessName;

/// When a service last settled a run of a process: saw how it ended,
/// stopped it, or saw it exit while unsupervised. An exit a holder left for
/// a later service, of a run that began before then, was settled already,
/// or overtaken by what the user did since, so it is never replayed.
#[derive(Debug, Clone, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct SettledRun {
    /// The process.
    process: ProcessName,
    /// When its run was settled.
    at: SystemTime,
}
