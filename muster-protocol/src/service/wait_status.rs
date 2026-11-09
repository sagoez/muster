use serde::{Deserialize, Serialize};
use strum::{Display, EnumIter, EnumMessage, EnumProperty};

/// The property naming how a person's terminal marks an ending: `ok`,
/// `hint` or `fail`.
pub const WAIT_KIND_PROPERTY: &str = "kind";

/// How a wait ended. Its message is how a person reads the ending, and its
/// [`WAIT_KIND_PROPERTY`] how a person's terminal marks it.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Display,
    EnumIter,
    EnumMessage,
    EnumProperty,
    Serialize,
    Deserialize,
)]
#[strum(serialize_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
pub enum WaitStatus {
    /// The condition holds.
    #[strum(message = "matched", props(kind = "ok"))]
    Matched,
    /// The time ran out first; waiting again from the cursor continues it.
    #[strum(message = "timed out", props(kind = "hint"))]
    Waiting,
    /// The process exited, status 0 or not, before the condition held.
    #[strum(message = "exited", props(kind = "fail"))]
    Exited,
    /// The process failed before the condition held.
    #[strum(message = "failed", props(kind = "fail"))]
    Crashed,
    /// The process was stopped before the condition held.
    #[strum(message = "stopped", props(kind = "fail"))]
    Stopped,
    /// The process was not running and no start was asked for.
    #[strum(message = "not running", props(kind = "fail"))]
    NotRunning,
    /// The run did not pass its readiness check in time; it keeps running.
    #[strum(message = "not ready in time", props(kind = "hint"))]
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
    use strum::{EnumMessage, EnumProperty, IntoEnumIterator};

    use super::{WAIT_KIND_PROPERTY, WaitStatus};

    /// Every ending has the words a person reads it as, and its mark.
    #[test]
    fn every_status_has_its_words_and_its_mark() {
        for status in WaitStatus::iter() {
            assert!(status.get_message().is_some(), "{status}");
            assert!(status.get_str(WAIT_KIND_PROPERTY).is_some(), "{status}");
        }
    }
}
