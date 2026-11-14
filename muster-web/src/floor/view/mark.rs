use serde::Serialize;

/// What sets a process apart from the others in its room.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Mark {
    /// Its child is suspended.
    Paused,
    /// Its child may run under a holder the service cannot reach.
    Muted,
    /// It is stopping, so its room shows no spinner for it.
    Quiet,
}
