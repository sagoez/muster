use crate::adapter::holder::LogScan;

/// What a read of a process's log found for one thing it was asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogAnswer {
    /// The scan of a held wait's slice of the log.
    Scan(LogScan),
    /// The last lines of a run, oldest first.
    Lines(Vec<String>),
}
