use serde::{Deserialize, Serialize};
use strum::{Display, EnumIter, EnumMessage};

/// How a wait ended. Its message is how a person reads the ending.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Display, EnumIter, EnumMessage, Serialize, Deserialize,
)]
#[strum(serialize_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
pub enum WaitStatus {
    /// The condition holds.
    #[strum(message = "matched")]
    Matched,
    /// The time ran out first; waiting again from the cursor continues it.
    #[strum(message = "timed out")]
    Waiting,
    /// The process exited, status 0 or not, before the condition held.
    #[strum(message = "exited")]
    Exited,
    /// The process failed before the condition held.
    #[strum(message = "failed")]
    Crashed,
    /// The process was stopped before the condition held.
    #[strum(message = "stopped")]
    Stopped,
    /// The process was not running and no start was asked for.
    #[strum(message = "not running")]
    NotRunning,
    /// The run did not pass its readiness check in time.
    #[strum(message = "not ready in time")]
    ReadyTimeout,
}

impl WaitStatus {
    /// Whether the wait ended in a way the caller must act on: anything but
    /// a match or a wait that ran out of time.
    pub fn is_failure(self) -> bool {
        !matches!(self, Self::Matched | Self::Waiting)
    }
}

#[cfg(test)]
mod tests {
    use strum::{EnumMessage, IntoEnumIterator};

    use super::WaitStatus;

    /// Every ending has the words a person reads it as.
    #[test]
    fn every_status_has_its_words() {
        for status in WaitStatus::iter() {
            assert!(status.get_message().is_some(), "{status}");
        }
    }
}
