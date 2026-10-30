use serde::{Deserialize, Serialize};
use strum::Display;

/// How a wait ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display, Serialize, Deserialize)]
#[strum(serialize_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
pub enum WaitStatus {
    /// The condition holds.
    Matched,
    /// The time ran out first; waiting again from the cursor continues it.
    Waiting,
    /// The process exited, status 0 or not, before the condition held.
    Exited,
    /// The process crashed before the condition held.
    Crashed,
    /// The process was stopped before the condition held.
    Stopped,
    /// The process was not running and no start was asked for.
    NotRunning,
    /// The run did not pass its readiness check in time.
    ReadyTimeout,
}

impl WaitStatus {
    /// Whether the wait ended in a way the caller must act on: anything but
    /// a match or a wait that ran out of time.
    pub fn is_failure(self) -> bool {
        !matches!(self, Self::Matched | Self::Waiting)
    }
}
