use std::collections::VecDeque;

use getset::CopyGetters;

use super::{log_query::LogQuery, log_reader::GROUP_SEPARATOR};
use crate::domain::value::OutputCursor;

/// The newest lines a read keeps, dropping the oldest past the line or byte
/// cap. Group separators take bytes but do not count as lines.
#[derive(CopyGetters)]
pub(super) struct Kept {
    /// The lines, oldest first, each with where it begins in the output.
    lines: VecDeque<(String, OutputCursor)>,
    /// Separators among the lines.
    separators: usize,
    /// Bytes the lines take with their line feeds.
    bytes: usize,
    /// Lines dropped to stay within the caps.
    #[getset(get_copy = "pub(super)")]
    omitted: usize,
    /// The most lines kept.
    line_cap: usize,
    /// The most bytes kept.
    byte_cap: usize,
    /// Whether a line dropped from now on counts as omitted.
    counting: bool,
    /// How many entries at the front were added while not counting, so
    /// dropping them omits nothing.
    uncounted: usize,
}

impl Kept {
    /// An empty collection with the caps of `query`.
    pub(super) fn new(query: &LogQuery) -> Self {
        Self {
            lines: VecDeque::new(),
            separators: 0,
            bytes: 0,
            omitted: 0,
            line_cap: query.lines().into_inner(),
            byte_cap: query.max_bytes().into_inner(),
            counting: true,
            uncounted: 0,
        }
    }

    /// Sets whether the lines added from now on count as omitted when the
    /// caps drop them: lines read only in case the newest wanted reach
    /// back further than estimated do not.
    pub(super) fn count_from_now(&mut self, counting: bool) {
        self.counting = counting;
    }

    /// Adds a line that passed the filter, which begins at `at`.
    pub(super) fn push(&mut self, line: String, at: OutputCursor) {
        self.bytes += line.len() + 1;
        self.lines.push_back((line, at));
        if !self.counting {
            self.uncounted += 1;
        }
        self.enforce_caps();
    }

    /// Drops the oldest entry, counting it as omitted unless it was added
    /// while not counting or `separator` says it is a group separator.
    fn drop_oldest(&mut self, separator: bool) {
        let Some((line, _)) = self.lines.pop_front() else {
            return;
        };
        self.bytes -= line.len() + 1;
        if self.uncounted > 0 {
            self.uncounted -= 1;
        } else if !separator {
            self.omitted += 1;
        }
    }

    /// Adds a separator before the error group whose first line begins at
    /// `at`, unless nothing precedes it.
    pub(super) fn separate(&mut self, at: OutputCursor) {
        if !self.lines.is_empty() {
            self.separators += 1;
            self.push(GROUP_SEPARATOR.to_string(), at);
        }
    }

    /// Where the oldest line kept begins, when any is.
    pub(super) fn first_at(&self) -> Option<OutputCursor> {
        self.lines.front().map(|(_, at)| *at)
    }

    /// Drops the oldest lines until the caps hold.
    fn enforce_caps(&mut self) {
        while self.lines.len() - self.separators > self.line_cap || self.bytes > self.byte_cap {
            if self.lines.is_empty() {
                break;
            }
            self.drop_oldest(false);
            if self
                .lines
                .front()
                .is_some_and(|(line, _)| line == GROUP_SEPARATOR)
            {
                self.drop_oldest(true);
                self.separators -= 1;
            }
        }
    }

    /// The kept lines, oldest first.
    pub(super) fn into_lines(self) -> Vec<String> {
        self.lines.into_iter().map(|(line, _)| line).collect()
    }
}
