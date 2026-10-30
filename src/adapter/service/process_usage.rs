use getset::Getters;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use crate::domain::value::Pid;

/// Resource usage of a running process. Carries the OS pid so the reading is
/// actionable (for `kill`, logs, ...), plus the tree-summed load.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
#[serde(rename_all = "kebab-case")]
pub struct ProcessUsage {
    /// Operating-system process id of the process.
    pid: Pid,
    /// Tree-summed CPU use, as a percentage where 100 is one core.
    cpu_percent: f32,
    /// Tree-summed resident memory, in bytes.
    memory_bytes: u64,
}
