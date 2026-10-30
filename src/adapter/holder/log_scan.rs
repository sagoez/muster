use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use super::logged_line::LoggedLine;
use crate::domain::value::OutputCursor;

/// What a scan of a log for one pattern found: the first matching line, and
/// where the scan began, past what rotation discarded.
#[derive(Debug, Clone, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub struct LogScan {
    /// Where the scan began: the cursor asked for, or the oldest byte the log
    /// still keeps when that is newer.
    #[getset(get_copy = "pub")]
    from: OutputCursor,
    /// The first line the pattern matched, if any.
    #[getset(get = "pub")]
    matched: Option<LoggedLine>,
}
