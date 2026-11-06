use crate::adapter::holder::{LogLines, LogScan};

/// What a read of a process's log found for one thing it was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogAnswer {
    /// The scan of a held wait's slice of the log.
    Scan(LogScan),
    /// The last lines of a run, oldest first.
    Lines(LogLines),
}

impl LogAnswer {
    /// Lines the read dropped because rendering them failed.
    pub fn unrendered_lines(&self) -> usize {
        match self {
            Self::Scan(scan) => scan.unrendered_lines(),
            Self::Lines(lines) => lines.unrendered_lines(),
        }
    }
}
