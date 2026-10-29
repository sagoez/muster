use std::{fmt, time::Duration};

use getset::Getters;
use serde::{Deserialize, Serialize};
use strum::{Display, EnumIter, EnumString};
use typed_builder::TypedBuilder;

use crate::domain::{
    process::{LogPattern, ProcessState},
    value::{LocalPort, LocalUrl, OutputCursor},
};

/// A lifecycle state a wait can be for: one `ps` shows, or `stopped` for not
/// running for any reason.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Display, EnumString, EnumIter, Serialize, Deserialize,
)]
#[strum(serialize_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
pub enum StateGoal {
    /// Running.
    Running,
    /// Paused.
    Paused,
    /// Exited with status 0.
    Exited,
    /// Crashed.
    Crashed,
    /// Waiting out a restart backoff.
    Restarting,
    /// Not running, whatever the reason.
    Stopped,
}

impl StateGoal {
    /// Whether a process in `state` has reached this goal.
    pub fn reached(self, state: ProcessState) -> bool {
        match self {
            Self::Running => state == ProcessState::Running,
            Self::Paused => state == ProcessState::Paused,
            Self::Exited => state == ProcessState::Exited,
            Self::Crashed => state == ProcessState::Crashed,
            Self::Restarting => state == ProcessState::Restarting,
            Self::Stopped => !state.may_be_running(),
        }
    }
}

/// What a wait waits for. One shape serves waits, starts with a wait, and
/// stored readiness.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WaitCondition {
    /// An output line matching the pattern.
    Log(LogPattern),
    /// A TCP connection accepted on the local port.
    Port(LocalPort),
    /// An HTTP answer from the local URL: `status` when given, else below 500.
    Http {
        /// What to fetch.
        url: LocalUrl,
        /// The status to wait for.
        status: Option<u16>,
    },
    /// The process reaching a lifecycle state.
    State(StateGoal),
    /// The process's stored readiness check passing.
    Ready,
}

impl WaitCondition {
    /// The condition a wait with none named waits for: readiness when the
    /// process has a check, else running.
    pub fn default_for(has_ready_check: bool) -> Self {
        if has_ready_check {
            Self::Ready
        } else {
            Self::State(StateGoal::Running)
        }
    }
}

impl fmt::Display for WaitCondition {
    /// Writes the condition for a person: `log "Listening on"`, `port 3000`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Log(pattern) => write!(formatter, "log \"{pattern}\""),
            Self::Port(port) => write!(formatter, "port {port}"),
            Self::Http {
                url,
                status: Some(status),
            } => write!(formatter, "http {url} ({status})"),
            Self::Http { url, status: None } => write!(formatter, "http {url}"),
            Self::State(goal) => write!(formatter, "state {goal}"),
            Self::Ready => formatter.write_str("ready"),
        }
    }
}

/// One wait as a client asks for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
#[serde(rename_all = "kebab-case")]
pub struct WaitSpec {
    /// What to wait for; the process's default when absent.
    #[builder(default)]
    until: Option<WaitCondition>,
    /// Output after this cursor counts; without it, only output after the
    /// request does.
    #[builder(default)]
    since: Option<OutputCursor>,
    /// Longest to wait before answering that it is still waiting.
    #[serde(with = "humantime_serde")]
    timeout: Duration,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopped_means_not_running_for_any_reason() {
        for state in [
            ProcessState::Pending,
            ProcessState::Exited,
            ProcessState::Crashed,
        ] {
            assert!(StateGoal::Stopped.reached(state), "{state}");
        }
        for state in [
            ProcessState::Running,
            ProcessState::Starting,
            ProcessState::Restarting,
            ProcessState::Unsupervised,
        ] {
            assert!(!StateGoal::Stopped.reached(state), "{state}");
        }
        assert!(StateGoal::Running.reached(ProcessState::Running));
        assert!(!StateGoal::Running.reached(ProcessState::Starting));
    }

    #[test]
    fn the_default_is_readiness_only_with_a_check() {
        assert_eq!(WaitCondition::default_for(true), WaitCondition::Ready);
        assert_eq!(
            WaitCondition::default_for(false),
            WaitCondition::State(StateGoal::Running)
        );
    }
}
