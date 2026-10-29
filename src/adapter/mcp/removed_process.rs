use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use crate::domain::value::ProcessName;

/// What `process_remove` did: the process left the stack, and whether a run
/// of it was stopped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct RemovedProcess {
    /// The process, as the stack spelt it.
    #[getset(get = "pub")]
    name: ProcessName,
    /// Whether it was running and has ended.
    #[getset(get_copy = "pub")]
    stopped: bool,
    /// Whether it was still stopping when the wait ended; it ends through its
    /// stop policy.
    #[getset(get_copy = "pub")]
    still_stopping: bool,
}
