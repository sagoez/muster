use std::time::Duration;

use getset::{CopyGetters, Getters};
use serde::{Deserialize, Serialize};
use strum::Display;
use typed_builder::TypedBuilder;

use super::wait_condition::WaitCondition;
use crate::domain::value::{OutputCursor, ProcessName};

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

/// The output line that satisfied a wait.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
#[serde(rename_all = "kebab-case")]
pub struct MatchedLine {
    /// The line, escapes removed.
    line: String,
    /// Where it began in the output.
    at: OutputCursor,
}

/// How one wait ended, with what the caller needs next.
#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, CopyGetters, TypedBuilder,
)]
#[serde(rename_all = "kebab-case")]
pub struct WaitOutcome {
    /// The process waited on.
    #[getset(get = "pub")]
    process: ProcessName,
    /// How it ended.
    #[getset(get_copy = "pub")]
    status: WaitStatus,
    /// What was waited for, the default resolved.
    #[getset(get = "pub")]
    until: WaitCondition,
    /// The line that matched, for a log condition that matched.
    #[getset(get = "pub")]
    #[builder(default)]
    matched: Option<MatchedLine>,
    /// Where the output stood when it ended, to wait or read on from.
    #[getset(get = "pub")]
    cursor: OutputCursor,
    /// How long it waited.
    #[getset(get_copy = "pub")]
    #[serde(with = "humantime_serde")]
    elapsed: Duration,
    /// The run's last lines, when the wait did not match.
    #[getset(get = "pub")]
    #[builder(default)]
    tail: Vec<String>,
    /// Where scanning began, when the asked cursor was older than the output
    /// still held.
    #[getset(get = "pub")]
    #[builder(default)]
    scanned_from: Option<OutputCursor>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_read_and_serialize_as_kebab_case() {
        assert_eq!(WaitStatus::NotRunning.to_string(), "not-running");
        assert_eq!(
            serde_json::to_string(&WaitStatus::ReadyTimeout).unwrap(),
            "\"ready-timeout\""
        );
    }
}
