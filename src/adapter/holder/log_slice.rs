use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use crate::domain::value::OutputCursor;

/// What one log read returns: stripped, filtered and capped lines, and the
/// span of output they were taken from.
#[derive(Debug, Clone, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub struct LogSlice {
    /// Where the examined output starts.
    #[getset(get_copy = "pub")]
    from: OutputCursor,
    /// The end of the examined output; reading again from it returns only
    /// newer output.
    #[getset(get_copy = "pub")]
    cursor: OutputCursor,
    /// The lines kept, oldest first; error groups are separated by `--`.
    #[getset(get = "pub")]
    lines: Vec<String>,
    /// Lines that passed the filter but were dropped, oldest first, to stay
    /// within the line and byte caps.
    #[getset(get_copy = "pub")]
    omitted_lines: usize,
    /// Bytes between the requested cursor and the oldest output still kept,
    /// which rotation discarded.
    #[getset(get_copy = "pub")]
    rotated_away_bytes: u64,
}
