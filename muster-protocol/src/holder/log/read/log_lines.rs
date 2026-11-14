use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

/// The last lines of part of a process's log, stripped, oldest first.
#[derive(Debug, Clone, Default, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub struct LogLines {
    /// The lines.
    #[getset(get = "pub")]
    lines: Vec<String>,
    /// Lines dropped because rendering them failed.
    #[getset(get_copy = "pub")]
    unrendered_lines: usize,
}

impl LogLines {
    /// The lines, without what was dropped.
    pub fn into_lines(self) -> Vec<String> {
        self.lines
    }
}
