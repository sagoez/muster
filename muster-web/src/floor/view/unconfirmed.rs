use serde::Serialize;

/// Why nothing confirms a process's status, so the page draws it dimmed,
/// as last known, and its card says why.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Unconfirmed {
    /// The service is absent, starting, of another version or not
    /// answering, while the child runs on under its holder.
    ServiceDown,
    /// No status has been read since the floor rested, nobody watching.
    Resting,
    /// Its project's stack does not read, so it may no longer hold it.
    UnreadableStack,
}
