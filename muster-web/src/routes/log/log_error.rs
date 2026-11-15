use serde::Serialize;

/// Why the log route read no lines.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum LogError {
    /// The floor holds no process by the names the request wrote.
    UnknownProcess,
    /// The process's log could not be read.
    UnreadableLog,
}
