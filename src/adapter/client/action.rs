use std::time::Duration;

use serde::Serialize;
use strum::{Display, EnumIter, EnumMessage};

use crate::{
    adapter::service::{ProcessAction, ProcessStatus},
    domain::process::ProcessState,
};

/// Longest a command waits for a launch to settle. Above a holder's 5 s
/// ready timeout and the 2 s to stop a holder that failed, so a launch that
/// fails is reported as failed rather than as still starting.
const LAUNCH_SETTLE_TIMEOUT: Duration = Duration::from_secs(15);
/// How long past a process's stop grace a command waits for its stop to
/// settle: the kill after the grace, the holder draining the child's last
/// output, and the exit reaching the service.
const STOP_SETTLE_MARGIN: Duration = Duration::from_secs(5);
/// Longest a command waits for a pause or resume asked for while the process
/// was starting.
const SUSPEND_SETTLE_TIMEOUT: Duration = Duration::from_secs(15);

/// What a command asked the service to do, as its report names it. Each
/// variant's message is the word for it while it is under way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, EnumIter, EnumMessage, Serialize)]
#[strum(serialize_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
pub enum Action {
    /// Start the project's `autostart` processes.
    #[strum(message = "starting")]
    Up,
    /// Stop the project's processes.
    #[strum(message = "stopping")]
    Down,
    /// Start one process.
    #[strum(message = "starting")]
    Start,
    /// Stop one process.
    #[strum(message = "stopping")]
    Stop,
    /// Restart one process.
    #[strum(message = "restarting")]
    Restart,
    /// Suspend one process.
    #[strum(message = "pausing")]
    Pause,
    /// Resume one suspended process.
    #[strum(message = "resuming")]
    Resume,
    /// Kill one process at once.
    #[strum(message = "killing")]
    Kill,
    /// Add a command and start it.
    #[strum(message = "starting")]
    Run,
    /// Remove processes from the project, stopping those that run.
    #[strum(message = "stopping")]
    Remove,
}

impl From<ProcessAction> for Action {
    fn from(action: ProcessAction) -> Self {
        match action {
            ProcessAction::Start => Self::Start,
            ProcessAction::Stop => Self::Stop,
            ProcessAction::Restart => Self::Restart,
            ProcessAction::Pause => Self::Pause,
            ProcessAction::Resume => Self::Resume,
            ProcessAction::Kill => Self::Kill,
        }
    }
}

impl Action {
    /// Whether the action ends processes.
    pub fn halts(self) -> bool {
        match self {
            Self::Down | Self::Stop | Self::Kill | Self::Remove => true,
            Self::Up | Self::Start | Self::Restart | Self::Pause | Self::Resume | Self::Run => {
                false
            },
        }
    }

    /// Whether `process` is still settling after this action: on its way, or
    /// launched so recently that it may yet exit at once, so a launch report
    /// sees a child that dies at once crash rather than start.
    pub fn settling(self, process: &ProcessStatus) -> bool {
        let launches = match self {
            Self::Up | Self::Start | Self::Restart | Self::Run => true,
            Self::Down | Self::Stop | Self::Kill | Self::Remove | Self::Pause | Self::Resume => {
                false
            },
        };
        self.in_transition(*process.state()) || (launches && process.in_quick_exit_window())
    }

    /// Whether a process in `state` is still on its way to what this action
    /// asked for, so the command keeps waiting before it reports. A removed
    /// process is on its way for as long as it may still run, since the
    /// service stops it once it hears of the removal.
    pub fn in_transition(self, state: ProcessState) -> bool {
        match state {
            ProcessState::Starting => true,
            ProcessState::Stopping => self.halts() || self == Self::Restart,
            ProcessState::Running
            | ProcessState::Paused
            | ProcessState::Restarting
            | ProcessState::Unsupervised => self == Self::Remove,
            ProcessState::Pending
            | ProcessState::Exited
            | ProcessState::Crashed
            | ProcessState::Blocked => false,
        }
    }

    /// Longest the command waits for the action to settle, for processes
    /// whose longest stop grace is `stop_grace`: a stop waits out the grace
    /// and a margin, a restart that and a launch.
    pub fn settle_timeout(self, stop_grace: Duration) -> Duration {
        let stop = stop_grace + STOP_SETTLE_MARGIN;
        match self {
            Self::Restart => stop + LAUNCH_SETTLE_TIMEOUT,
            Self::Down | Self::Stop | Self::Kill | Self::Remove => stop,
            Self::Up | Self::Start | Self::Run => LAUNCH_SETTLE_TIMEOUT,
            Self::Pause | Self::Resume => SUSPEND_SETTLE_TIMEOUT,
        }
    }
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

    use super::*;

    #[test]
    fn a_launch_waits_out_starting_and_a_restart_its_stop_too() {
        assert!(Action::Start.in_transition(ProcessState::Starting));
        assert!(!Action::Start.in_transition(ProcessState::Running));
        assert!(!Action::Start.in_transition(ProcessState::Crashed));
        assert!(Action::Restart.in_transition(ProcessState::Stopping));
        assert!(Action::Stop.in_transition(ProcessState::Stopping));
        assert!(Action::Remove.in_transition(ProcessState::Stopping));
        assert!(Action::Remove.in_transition(ProcessState::Running));
        assert!(!Action::Remove.in_transition(ProcessState::Exited));
        assert!(!Action::Stop.in_transition(ProcessState::Running));
        assert!(Action::Stop.in_transition(ProcessState::Starting));
        assert!(!Action::Pause.in_transition(ProcessState::Stopping));
    }

    /// A stop waits for the process's own grace period, however long it is
    /// configured, plus the margin.
    #[test]
    fn a_stop_waits_out_the_configured_grace() {
        let grace = Duration::from_secs(60);

        assert_eq!(
            Action::Stop.settle_timeout(grace),
            grace + STOP_SETTLE_MARGIN
        );
        assert_eq!(
            Action::Remove.settle_timeout(grace),
            grace + STOP_SETTLE_MARGIN
        );
        assert_eq!(
            Action::Restart.settle_timeout(grace),
            grace + STOP_SETTLE_MARGIN + LAUNCH_SETTLE_TIMEOUT
        );
        assert_eq!(Action::Start.settle_timeout(grace), LAUNCH_SETTLE_TIMEOUT);
    }

    /// Every action has the one word a spinner shows while it is under way.
    #[test]
    fn each_action_has_a_word() {
        for action in Action::iter() {
            assert!(action.get_message().is_some(), "{action}");
        }
        assert_eq!(Action::Up.get_message(), Some("starting"));
        assert_eq!(Action::Down.get_message(), Some("stopping"));
        assert_eq!(Action::Restart.get_message(), Some("restarting"));
    }

    #[test]
    fn actions_are_named_in_kebab_case() {
        assert_eq!(Action::Remove.to_string(), "remove");
        assert_eq!(serde_json::to_string(&Action::Run).unwrap(), "\"run\"");
    }
}
