use std::fmt;

use serde::{Deserialize, Serialize};

use super::state_goal::StateGoal;
use crate::domain::{
    process::LogPattern,
    value::{LocalPort, LocalUrl},
};

/// What a wait waits for. One shape serves waits, starts with a wait, and
/// stored readiness.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WaitCondition {
    /// An output line matching the pattern.
    Log(#[serde(with = "crate::adapter::wire::form")] LogPattern),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::process::ProcessState;

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
