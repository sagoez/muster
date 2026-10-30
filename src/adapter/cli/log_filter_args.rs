use getset::{CopyGetters, Getters};

use super::{
    args::{RAW_LOG_FLAGS, line_limit},
    error::CliError,
};
use crate::{
    adapter::holder::{LineFilter, LineLimit, LineMatcher, LogQuery},
    domain::{process::PatternKind, value::OutputCursor},
};

/// `muster logs` flags that read the log as stripped, filtered lines rather
/// than the raw bytes; any one of them switches to that read.
#[derive(clap::Args, Getters, CopyGetters)]
pub struct LogFilterArgs {
    /// Print only the newest N lines, escapes removed (at most 2000).
    #[arg(short = 'n', long, value_name = "N", value_parser = line_limit, conflicts_with_all = RAW_LOG_FLAGS)]
    #[getset(get = "pub")]
    lines: Option<LineLimit>,
    /// Print only output after this cursor, from an earlier `muster wait` or
    /// `muster logs --json`.
    #[arg(long, value_name = "CURSOR", conflicts_with_all = RAW_LOG_FLAGS)]
    #[getset(get = "pub")]
    since: Option<OutputCursor>,
    /// Print only lines containing this text.
    #[arg(long, value_name = "TEXT", conflicts_with_all = RAW_LOG_FLAGS)]
    #[getset(get = "pub")]
    grep: Option<String>,
    /// Read --grep as a regular expression.
    #[arg(long, requires = "grep")]
    #[getset(get_copy = "pub")]
    regex: bool,
    /// Print only error lines, each with its stack trace.
    #[arg(long, conflicts_with = "grep", conflicts_with_all = RAW_LOG_FLAGS)]
    #[getset(get_copy = "pub")]
    errors: bool,
}

impl LogFilterArgs {
    /// Whether any filter flag was given.
    pub fn is_empty(&self) -> bool {
        self.lines.is_none() && self.since.is_none() && self.grep.is_none() && !self.errors
    }

    /// The log read these flags ask for, its pattern compiled first.
    ///
    /// # Errors
    /// Returns [`CliError::InvalidPattern`] for a blank or overlong `--grep`
    /// or a `--regex` that does not compile.
    pub fn query(&self) -> Result<LogQuery, CliError> {
        let filter = match &self.grep {
            Some(grep) => LineFilter::Grep(LineMatcher::of_text(
                grep,
                PatternKind::of_regex(self.regex),
            )?),
            None if self.errors => LineFilter::ErrorsOnly,
            None => LineFilter::All,
        };
        Ok(LogQuery::builder()
            .since(self.since)
            .lines(self.lines.unwrap_or_default())
            .filter(filter)
            .build())
    }
}
