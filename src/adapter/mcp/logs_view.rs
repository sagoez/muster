use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use crate::domain::value::ProcessName;

/// A slice of a process's log as `process_logs` answers it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct LogsView {
    /// The process.
    #[getset(get = "pub")]
    process: ProcessName,
    /// Where the returned lines begin.
    #[getset(get = "pub")]
    from: String,
    /// The end of what was examined; pass it as since next time.
    #[getset(get = "pub")]
    cursor: String,
    /// Where the current run's output begins.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[getset(get = "pub")]
    run_start: Option<String>,
    /// How many lines are returned.
    #[getset(get_copy = "pub")]
    lines_returned: usize,
    /// How many earlier lines were dropped to fit the caps.
    #[getset(get_copy = "pub")]
    omitted_lines: usize,
    /// Bytes after since that rotation already removed.
    #[getset(get_copy = "pub")]
    rotated_away_bytes: u64,
    /// The lines, stripped of terminal escapes.
    #[getset(get = "pub")]
    text: String,
}
