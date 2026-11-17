use serde_json::Value;

/// One of Claude Code's servers files as a command read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ServersFile {
    /// There is no such file.
    Missing,
    /// It was read, as this JSON.
    Parsed(Value),
    /// It could not be read or parsed.
    Unreadable,
}
