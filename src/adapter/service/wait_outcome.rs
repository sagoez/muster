use std::time::Duration;

use getset::{CopyGetters, Getters};
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::{matched_line::MatchedLine, wait_condition::WaitCondition, wait_status::WaitStatus};
use crate::domain::value::{OutputCursor, ProcessName};

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
