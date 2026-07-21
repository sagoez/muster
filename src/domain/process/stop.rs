use std::time::Duration;

use getset::{Getters, WithSetters};
use serde::{Deserialize, Serialize};
use strum::Display;
use typed_builder::TypedBuilder;

/// Default time a command may spend shutting down before force-kill.
const DEFAULT_GRACE_PERIOD: Duration = Duration::from_secs(5);

/// Graceful signal sent before a command is force-killed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum StopSignal {
    /// Requests the conventional service shutdown path (SIGTERM on Unix).
    Terminate,
    /// Requests the conventional interactive interrupt path (SIGINT on Unix).
    Interrupt,
}

/// Graceful shutdown policy for one command process.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Getters, WithSetters, TypedBuilder,
)]
#[set_with]
pub struct StopPolicy {
    /// Graceful signal sent to the command's process group.
    #[getset(get = "pub", set_with = "pub")]
    signal: StopSignal,
    /// Time allowed for graceful exit before an unconditional force-kill.
    #[serde(with = "humantime_serde")]
    #[getset(get = "pub", set_with = "pub")]
    grace_period: Duration,
}

impl Default for StopPolicy {
    /// Returns the default command shutdown policy: SIGTERM with five seconds
    /// available for cleanup.
    fn default() -> Self {
        Self::builder()
            .signal(StopSignal::Terminate)
            .grace_period(DEFAULT_GRACE_PERIOD)
            .build()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Human-readable durations deserialize into the domain policy.
    #[test]
    fn parses_a_human_readable_grace_period() {
        let policy: StopPolicy =
            serde_yaml_ng::from_str("signal: terminate\ngrace_period: 5s\n").unwrap();

        assert_eq!(*policy.signal(), StopSignal::Terminate);
        assert_eq!(*policy.grace_period(), Duration::from_secs(5));
    }

    /// Commands default to a conventional service shutdown request.
    #[test]
    fn defaults_to_terminate_with_a_bounded_grace_period() {
        let policy = StopPolicy::default();

        assert_eq!(*policy.signal(), StopSignal::Terminate);
        assert_eq!(*policy.grace_period(), DEFAULT_GRACE_PERIOD);
    }
}
