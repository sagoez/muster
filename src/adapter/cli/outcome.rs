use serde::Serialize;
use strum::{Display, EnumMessage};

use super::action::Action;
use crate::{adapter::service::ProcessStatus, domain::process::ProcessState};

/// What became of one process a command acted on, once it settled or the
/// command stopped waiting. Each variant's message is how a person reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, EnumMessage, Serialize)]
#[strum(serialize_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// A new child is running.
    #[strum(message = "running")]
    Started,
    /// The old child was replaced by a new running one.
    #[strum(message = "restarted")]
    Restarted,
    /// The child that was running before is still running.
    #[strum(message = "already running")]
    AlreadyRunning,
    /// It ran and exited successfully.
    #[strum(message = "ran and exited")]
    Exited,
    /// It failed, or is crash-looping under its restart policy.
    #[strum(message = "crashed")]
    Crashed,
    /// A child the service cannot reach may still run, so it was not started.
    #[strum(message = "not started: it may still be running unsupervised")]
    Unsupervised,
    /// It was still starting or stopping when the command stopped waiting.
    #[strum(message = "still starting")]
    StillStarting,
    /// Nothing was launched; a notice says why.
    #[strum(message = "not started")]
    NotStarted,
    /// Its child has gone.
    #[strum(message = "stopped")]
    Stopped,
    /// Nothing was running to stop.
    #[strum(message = "was not running")]
    WasNotRunning,
    /// It was still stopping when the command stopped waiting.
    #[strum(message = "still stopping")]
    StillStopping,
    /// Its child kept running although it was asked to end, or to be
    /// replaced by a restart.
    #[strum(message = "still running")]
    StillRunning,
    /// It is suspended.
    #[strum(message = "paused")]
    Paused,
    /// It was suspended already.
    #[strum(message = "already paused")]
    AlreadyPaused,
    /// It runs again after a pause.
    #[strum(message = "resumed")]
    Resumed,
    /// The action did not apply to it in its state.
    #[strum(message = "nothing to do while it is")]
    Unchanged,
    /// It reopens once its previous run has exited.
    #[strum(message = "queued: it reopens once its previous run has exited")]
    Queued,
    /// The service left it alone, for the reason the report carries.
    #[strum(message = "left alone")]
    Refused,
    /// An agent `down` leaves running because `--agents` was not given.
    #[strum(message = "kept running")]
    KeptRunning,
    /// It is no longer listed.
    #[strum(message = "no longer listed")]
    Gone,
}

impl Outcome {
    /// What `action` came to for one process: listed as `before` when it was
    /// sent and as `after` once it settled, where `saw_transition` says
    /// whether the command saw the process starting or stopping in between.
    pub fn settle(
        action: Action,
        before: Option<&ProcessStatus>,
        after: Option<&ProcessStatus>,
        saw_transition: bool,
    ) -> Self {
        let was = before.map(|status| *status.state());
        let Some(after) = after else {
            return if action.halts() {
                Self::Stopped
            } else {
                Self::Gone
            };
        };
        let state = *after.state();
        if action.halts() {
            return halted(was, state);
        }
        if action.in_transition(state) {
            return match state {
                ProcessState::Stopping => Self::StillStopping,
                _ => Self::StillStarting,
            };
        }
        match action {
            Action::Pause => paused(was, state),
            Action::Resume => resumed(was, state),
            _ => launched(action, before, after, saw_transition),
        }
    }

    /// Whether the process did not end up as the action asked, which makes
    /// the command fail.
    pub fn is_failure(self) -> bool {
        matches!(
            self,
            Self::Crashed
                | Self::Unsupervised
                | Self::StillStarting
                | Self::NotStarted
                | Self::StillStopping
                | Self::StillRunning
                | Self::Unchanged
                | Self::Refused
                | Self::Gone
        )
    }

    /// Whether the outcome is worth a second look without being a failure.
    pub fn is_pending(self) -> bool {
        matches!(self, Self::Queued)
    }

    /// Whether the process was still on its way when the wait ended.
    pub fn is_unsettled(self) -> bool {
        matches!(self, Self::StillStarting | Self::StillStopping)
    }
}

/// The outcome of a stop, kill, close or down for a process that was `was`
/// and is now `state`.
fn halted(was: Option<ProcessState>, state: ProcessState) -> Outcome {
    if !was.is_some_and(ProcessState::may_be_running) {
        return Outcome::WasNotRunning;
    }
    match state {
        ProcessState::Starting | ProcessState::Stopping => Outcome::StillStopping,
        state if state.may_be_running() => Outcome::StillRunning,
        _ => Outcome::Stopped,
    }
}

/// The outcome of a pause for a process that was `was` and is now `state`.
fn paused(was: Option<ProcessState>, state: ProcessState) -> Outcome {
    match (was, state) {
        (Some(ProcessState::Paused), ProcessState::Paused) => Outcome::AlreadyPaused,
        (_, ProcessState::Paused) => Outcome::Paused,
        _ => Outcome::Unchanged,
    }
}

/// The outcome of a resume for a process that was `was` and is now `state`.
fn resumed(was: Option<ProcessState>, state: ProcessState) -> Outcome {
    match (was, state) {
        (Some(ProcessState::Running), ProcessState::Running) => Outcome::AlreadyRunning,
        (_, ProcessState::Running) => Outcome::Resumed,
        _ => Outcome::Unchanged,
    }
}

/// The outcome of a launch that left the process as `after`.
fn launched(
    action: Action,
    before: Option<&ProcessStatus>,
    after: &ProcessStatus,
    saw_transition: bool,
) -> Outcome {
    let same_child = before.is_some_and(|before| {
        before.pid().is_some() && before.pid() == after.pid() && before.state().is_active()
    });
    let changed = saw_transition
        || before.is_none_or(|before| {
            before.state() != after.state()
                || before.pid() != after.pid()
                || before.restarts() != after.restarts()
        });
    match after.state() {
        ProcessState::Running | ProcessState::Paused if same_child => match action {
            Action::Restart => Outcome::StillRunning,
            _ => Outcome::AlreadyRunning,
        },
        ProcessState::Running | ProcessState::Paused => match action {
            Action::Restart => Outcome::Restarted,
            _ => Outcome::Started,
        },
        ProcessState::Unsupervised => Outcome::Unsupervised,
        _ if !changed => Outcome::NotStarted,
        ProcessState::Exited => Outcome::Exited,
        ProcessState::Crashed | ProcessState::Restarting => Outcome::Crashed,
        _ => Outcome::NotStarted,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::domain::{
        process::{ActivityState, ProcessKind},
        value::ProcessName,
    };

    /// `api` listed in `state` with `pid` after `restarts` restarts.
    fn status(state: ProcessState, pid: Option<u32>, restarts: u32) -> ProcessStatus {
        ProcessStatus::builder()
            .name(ProcessName::try_new("api").unwrap())
            .kind(ProcessKind::Command)
            .state(state)
            .activity(ActivityState::Idle)
            .pid(pid)
            .uptime(pid.map(|_| Duration::from_secs(1)))
            .restarts(restarts)
            .usage(None)
            .attached(0)
            .last_exit(None)
            .stop_grace(None)
            .build()
    }

    #[test]
    fn a_launch_reports_what_the_process_became() {
        let stopped = status(ProcessState::Exited, None, 0);
        let settle = |after: ProcessStatus, saw| {
            Outcome::settle(Action::Start, Some(&stopped), Some(&after), saw)
        };

        assert_eq!(
            settle(status(ProcessState::Running, Some(7), 0), true),
            Outcome::Started
        );
        assert_eq!(
            settle(status(ProcessState::Crashed, None, 0), true),
            Outcome::Crashed
        );
        assert_eq!(
            settle(status(ProcessState::Restarting, None, 1), true),
            Outcome::Crashed
        );
        assert_eq!(
            settle(status(ProcessState::Exited, None, 0), true),
            Outcome::Exited
        );
        assert_eq!(
            settle(status(ProcessState::Exited, None, 0), false),
            Outcome::NotStarted,
            "a start that changed nothing launched nothing"
        );
        assert_eq!(
            settle(status(ProcessState::Unsupervised, Some(9), 0), false),
            Outcome::Unsupervised
        );
        assert_eq!(
            settle(status(ProcessState::Starting, None, 0), true),
            Outcome::StillStarting
        );
    }

    #[test]
    fn starting_a_running_process_leaves_it_and_a_restart_replaces_it() {
        let running = status(ProcessState::Running, Some(7), 0);

        assert_eq!(
            Outcome::settle(Action::Start, Some(&running), Some(&running), false),
            Outcome::AlreadyRunning
        );
        assert_eq!(
            Outcome::settle(Action::Restart, Some(&running), Some(&running), false),
            Outcome::StillRunning
        );
        assert_eq!(
            Outcome::settle(
                Action::Restart,
                Some(&running),
                Some(&status(ProcessState::Running, Some(8), 1)),
                true
            ),
            Outcome::Restarted
        );
    }

    #[test]
    fn a_halt_reports_whether_anything_was_running_and_whether_it_ended() {
        let running = status(ProcessState::Running, Some(7), 0);
        let exited = status(ProcessState::Exited, None, 0);

        assert_eq!(
            Outcome::settle(Action::Stop, Some(&running), Some(&exited), true),
            Outcome::Stopped
        );
        assert_eq!(
            Outcome::settle(Action::Stop, Some(&exited), Some(&exited), false),
            Outcome::WasNotRunning
        );
        assert_eq!(
            Outcome::settle(
                Action::Stop,
                Some(&running),
                Some(&status(ProcessState::Stopping, Some(7), 0)),
                true
            ),
            Outcome::StillStopping
        );
        assert_eq!(
            Outcome::settle(Action::AgentClose, Some(&running), None, true),
            Outcome::Stopped,
            "a closed session's row retires"
        );
        assert!(
            Outcome::StillStopping.is_failure(),
            "a stop that has not finished when the wait ends fails"
        );
        assert!(Outcome::StillRunning.is_failure());
    }

    /// A restart whose old process has not stopped by the end of the wait
    /// says it is still stopping, not still starting.
    #[test]
    fn a_restart_stuck_stopping_says_so() {
        let running = status(ProcessState::Running, Some(7), 0);

        assert_eq!(
            Outcome::settle(
                Action::Restart,
                Some(&running),
                Some(&status(ProcessState::Stopping, Some(7), 0)),
                true
            ),
            Outcome::StillStopping
        );
    }

    #[test]
    fn pause_and_resume_report_the_suspension() {
        let running = status(ProcessState::Running, Some(7), 0);
        let paused = status(ProcessState::Paused, Some(7), 0);
        let exited = status(ProcessState::Exited, None, 0);

        assert_eq!(
            Outcome::settle(Action::Pause, Some(&running), Some(&paused), false),
            Outcome::Paused
        );
        assert_eq!(
            Outcome::settle(Action::Pause, Some(&exited), Some(&exited), false),
            Outcome::Unchanged
        );
        assert_eq!(
            Outcome::settle(Action::Resume, Some(&paused), Some(&running), false),
            Outcome::Resumed
        );
    }
}
