use std::{io, path::Path};

use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use super::{output_cursor::OutputCursor, wait::LogPattern};
use crate::adapter::holder::HolderError;

/// Why the stand-in log reader reads nothing.
const LOG_READER: &str = "reading logs by cursor needs package A's log reader";

/// A compiled substring or regex over one line. Stand-in for package A's
/// `LineMatcher`.
#[derive(Debug, Clone, Getters)]
pub struct LineMatcher {
    /// The pattern it matches.
    #[getset(get = "pub")]
    pattern: LogPattern,
}

impl LineMatcher {
    /// Compiles `pattern`.
    ///
    /// # Errors
    /// Package A refuses a regex that does not compile; this stand-in
    /// accepts every pattern.
    pub fn compile(pattern: &LogPattern) -> Result<Self, HolderError> {
        Ok(Self {
            pattern: pattern.clone(),
        })
    }
}

/// Which lines a log read keeps. Stand-in for package A's `LineFilter`.
#[derive(Debug, Clone)]
pub enum LineFilter {
    /// Every line.
    All,
    /// Lines the matcher matches.
    Grep(LineMatcher),
    /// Error lines and their stack traces.
    ErrorsOnly,
}

/// One log read. Stand-in for package A's `LogQuery`.
#[derive(Debug, Clone, Getters, CopyGetters, TypedBuilder)]
pub struct LogQuery {
    /// Only output after this.
    #[getset(get_copy = "pub")]
    since: Option<OutputCursor>,
    /// Most lines returned.
    #[getset(get_copy = "pub")]
    lines: usize,
    /// Which lines are kept.
    #[getset(get = "pub")]
    filter: LineFilter,
    /// Most bytes returned.
    #[getset(get_copy = "pub")]
    max_bytes: usize,
}

/// What a log read returned. Stand-in for package A's `LogSlice`.
#[derive(Debug, Clone, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub struct LogSlice {
    /// Where the returned lines begin.
    #[getset(get_copy = "pub")]
    from: OutputCursor,
    /// The end of what was examined.
    #[getset(get_copy = "pub")]
    cursor: OutputCursor,
    /// The lines, stripped of escapes, oldest first.
    #[getset(get = "pub")]
    lines: Vec<String>,
    /// Lines dropped to fit the caps.
    #[getset(get_copy = "pub")]
    omitted_lines: usize,
    /// Bytes after `since` that rotation already removed.
    #[getset(get_copy = "pub")]
    rotated_away_bytes: u64,
}

/// Reads `log` as `query` asks. Stand-in for package A's `read_log`.
///
/// # Errors
/// Always an unsupported read until package A lands.
pub fn read_log(_log: &Path, _query: &LogQuery) -> Result<LogSlice, HolderError> {
    Err(HolderError::Io(io::Error::new(
        io::ErrorKind::Unsupported,
        LOG_READER,
    )))
}
