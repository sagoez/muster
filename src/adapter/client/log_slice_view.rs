use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use crate::{
    adapter::holder::LogSlice,
    domain::value::{OutputCursor, ProcessName},
};

/// Joins the lines of a slice into its text.
const LINE_FEED: &str = "\n";

/// A slice of a process's log as `muster logs --json` and `process_logs`
/// show it: the stripped lines as one text, and the cursors around them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct LogSliceView {
    /// The process.
    #[getset(get = "pub")]
    process: ProcessName,
    /// Where the returned output starts: the first line kept when the caps
    /// dropped older ones, otherwise where the read began.
    #[getset(get_copy = "pub")]
    from: OutputCursor,
    /// The end of what was examined; reading from it returns only newer
    /// output.
    #[getset(get_copy = "pub")]
    cursor: OutputCursor,
    /// Where the current run's output begins, while the process runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[getset(get_copy = "pub")]
    run_start: Option<OutputCursor>,
    /// How many lines are returned.
    #[getset(get_copy = "pub")]
    lines_returned: usize,
    /// How many earlier lines were dropped to fit the caps.
    #[getset(get_copy = "pub")]
    omitted_lines: usize,
    /// Bytes after the cursor asked for that rotation already removed.
    #[getset(get_copy = "pub")]
    rotated_away_bytes: u64,
    /// The lines, stripped of terminal escapes.
    #[getset(get = "pub")]
    text: String,
}

impl LogSliceView {
    /// `slice` of the log of `process`, whose current run began at
    /// `run_start`.
    pub fn for_slice(
        process: ProcessName,
        slice: &LogSlice,
        run_start: Option<OutputCursor>,
    ) -> Self {
        Self {
            process,
            from: slice.from(),
            cursor: slice.cursor(),
            run_start,
            lines_returned: slice.lines().len(),
            omitted_lines: slice.omitted_lines(),
            rotated_away_bytes: slice.rotated_away_bytes(),
            text: slice.lines().join(LINE_FEED),
        }
    }
}
