use serde::{Deserialize, Serialize};
use strum::Display;

/// Lifecycle state of a managed process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Display, Serialize, Deserialize)]
#[strum(serialize_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
pub enum ProcessState {
    /// Configured but not yet started.
    #[default]
    Pending,
    /// Launch requested, holder not yet connected: a new process is being
    /// launched, or a running one adopted.
    Starting,
    /// Child process is alive.
    Running,
    /// Child process is alive but suspended (SIGSTOP).
    Paused,
    /// A stop or restart signal was delivered and exit is pending.
    Stopping,
    /// Exited successfully (status 0).
    Exited,
    /// Exited with a failure status or was killed by a signal.
    Crashed,
    /// Scheduled to restart after a backoff.
    Restarting,
    /// The process may still be running under a holder the service cannot
    /// reach, so it is neither supervised nor safe to start again.
    Unsupervised,
    /// Its last start was refused because a port it needs is taken; nothing
    /// runs until a later start finds the port free.
    Blocked,
}

impl ProcessState {
    /// Whether the process currently has a live child attached, or one being
    /// connected, so starting it again would put a second copy beside it.
    pub fn is_active(self) -> bool {
        matches!(
            self,
            Self::Starting | Self::Running | Self::Paused | Self::Stopping | Self::Restarting
        )
    }

    /// Whether a child of this process may still be running, supervised or
    /// not, so stopping it is meaningful.
    pub fn may_be_running(self) -> bool {
        self.is_active() || self == Self::Unsupervised
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_pending() {
        assert_eq!(ProcessState::default(), ProcessState::Pending);
    }

    #[test]
    fn active_while_a_child_is_live_or_transitioning() {
        assert!(ProcessState::Starting.is_active());
        assert!(ProcessState::Running.is_active());
        assert!(ProcessState::Paused.is_active());
        assert!(ProcessState::Stopping.is_active());
        assert!(ProcessState::Restarting.is_active());
        assert!(!ProcessState::Pending.is_active());
        assert!(!ProcessState::Exited.is_active());
        assert!(!ProcessState::Crashed.is_active());
        assert!(!ProcessState::Unsupervised.is_active());
    }

    #[test]
    fn an_unsupervised_process_may_still_be_running() {
        assert!(ProcessState::Unsupervised.may_be_running());
        assert!(ProcessState::Running.may_be_running());
        assert!(!ProcessState::Exited.may_be_running());
        assert_eq!(ProcessState::Unsupervised.to_string(), "unsupervised");
        assert_eq!(ProcessState::Starting.to_string(), "starting");
    }
}
