use std::time::Duration;

use serde::Serialize;
use strum::{Display, EnumMessage};

use crate::{adapter::service::ProcessAction, domain::process::ProcessState};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, EnumMessage, Serialize)]
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
    /// Register a command and start it.
    #[strum(message = "starting")]
    Run,
    /// Open a new agent session.
    #[strum(message = "starting")]
    AgentNew,
    /// Close an agent session.
    #[strum(message = "stopping")]
    AgentClose,
    /// Reopen the most recently closed agent session.
    #[strum(message = "starting")]
    AgentReopen,
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
    /// Whether the action launches processes.
    pub fn launches(self) -> bool {
        matches!(
            self,
            Self::Up | Self::Start | Self::Restart | Self::Run | Self::AgentNew | Self::AgentReopen
        )
    }

    /// Whether the action ends processes.
    pub fn halts(self) -> bool {
        matches!(
            self,
            Self::Down | Self::Stop | Self::Kill | Self::AgentClose
        )
    }

    /// Whether a process in `state` is still on its way to what this action
    /// asked for, so the command keeps waiting before it reports.
    pub fn in_transition(self, state: ProcessState) -> bool {
        match state {
            ProcessState::Starting => true,
            ProcessState::Stopping => self.halts() || self == Self::Restart,
            _ => false,
        }
    }

    /// Longest the command waits for the action to settle, for processes
    /// whose longest stop grace is `stop_grace`: a stop waits out the grace
    /// and a margin, a restart that and a launch.
    pub fn settle_timeout(self, stop_grace: Duration) -> Duration {
        let stop = stop_grace + STOP_SETTLE_MARGIN;
        match self {
            Self::Restart => stop + LAUNCH_SETTLE_TIMEOUT,
            action if action.halts() => stop,
            action if action.launches() => LAUNCH_SETTLE_TIMEOUT,
            _ => SUSPEND_SETTLE_TIMEOUT,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_launch_waits_out_starting_and_a_restart_its_stop_too() {
        assert!(Action::Start.in_transition(ProcessState::Starting));
        assert!(!Action::Start.in_transition(ProcessState::Running));
        assert!(!Action::Start.in_transition(ProcessState::Crashed));
        assert!(Action::Restart.in_transition(ProcessState::Stopping));
        assert!(Action::Stop.in_transition(ProcessState::Stopping));
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
            Action::Restart.settle_timeout(grace),
            grace + STOP_SETTLE_MARGIN + LAUNCH_SETTLE_TIMEOUT
        );
        assert_eq!(Action::Start.settle_timeout(grace), LAUNCH_SETTLE_TIMEOUT);
    }

    /// Each action has the one word a spinner shows while it is under way.
    #[test]
    fn each_action_has_a_word_while_under_way() {
        assert_eq!(Action::Up.get_message(), Some("starting"));
        assert_eq!(Action::Down.get_message(), Some("stopping"));
        assert_eq!(Action::Restart.get_message(), Some("restarting"));
    }

    #[test]
    fn actions_are_named_in_kebab_case() {
        assert_eq!(Action::AgentReopen.to_string(), "agent-reopen");
        assert_eq!(
            serde_json::to_string(&Action::AgentNew).unwrap(),
            "\"agent-new\""
        );
    }
}
