use super::{adopter::HolderAdopter, log_answer::LogAnswer};
use crate::{
    adapter::holder::LineMatcher,
    domain::value::{OutputCursor, ProcessName, ProjectRoot},
};

/// One thing a read of a process's log is asked for a wait held on it.
pub(super) enum LogAsk {
    /// The first line from `since` up to `until` that `matcher` matches, for
    /// a wait whose cursor is older than what the tap holds.
    Scan {
        /// Where the wait's cursor stands.
        since: OutputCursor,
        /// Where the tap's lines begin.
        until: OutputCursor,
        /// The wait's pattern.
        matcher: LineMatcher,
    },
    /// The last `count` lines from `from` up to `until`, for the answer of a
    /// wait whose run the tap holds too little of.
    Lines {
        /// Where the run began.
        from: OutputCursor,
        /// Where its output stood.
        until: OutputCursor,
        /// How many lines the answer shows.
        count: usize,
    },
}

impl LogAsk {
    /// Reads what this asks of the log of the process `name` of `project`
    /// through `adopter`; none when the log cannot be read.
    pub(super) fn answer(
        &self,
        adopter: &dyn HolderAdopter,
        project: &ProjectRoot,
        name: &ProcessName,
    ) -> Option<LogAnswer> {
        match self {
            Self::Scan {
                since,
                until,
                matcher,
            } => adopter
                .log_scan(project, name, *since, *until, matcher)
                .map(LogAnswer::Scan),
            Self::Lines { from, until, count } => adopter
                .log_lines(project, name, *from, *until, *count)
                .map(LogAnswer::Lines),
        }
    }
}
