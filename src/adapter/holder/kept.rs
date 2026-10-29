use std::collections::VecDeque;

use getset::CopyGetters;

use super::{log_query::LogQuery, log_reader::GROUP_SEPARATOR};

/// The newest lines a read keeps, dropping the oldest past the line or byte
/// cap. Group separators take bytes but do not count as lines.
#[derive(CopyGetters)]
pub(super) struct Kept {
    /// The lines, oldest first.
    lines: VecDeque<String>,
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
        }
    }

    /// Adds a line that passed the filter.
    pub(super) fn push(&mut self, line: String) {
        self.bytes += line.len() + 1;
        self.lines.push_back(line);
        self.enforce_caps();
    }

    /// Adds a separator between error groups, unless nothing precedes it.
    pub(super) fn separate(&mut self) {
        if !self.lines.is_empty() {
            self.separators += 1;
            self.push(GROUP_SEPARATOR.to_string());
        }
    }

    /// Drops the oldest lines until the caps hold.
    fn enforce_caps(&mut self) {
        while self.lines.len() - self.separators > self.line_cap || self.bytes > self.byte_cap {
            if let Some(line) = self.lines.pop_front() {
                self.bytes -= line.len() + 1;
                self.omitted += 1;
            }
            if self
                .lines
                .front()
                .is_some_and(|line| line == GROUP_SEPARATOR)
            {
                self.lines.pop_front();
                self.separators -= 1;
                self.bytes -= GROUP_SEPARATOR.len() + 1;
            }
        }
    }

    /// The kept lines, oldest first.
    pub(super) fn into_lines(self) -> Vec<String> {
        self.lines.into()
    }
}
