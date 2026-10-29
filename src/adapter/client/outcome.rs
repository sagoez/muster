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
    /// The service left it alone, for the reason the report carries.
    #[strum(message = "left alone")]
    Refused,
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
        if action.halts() {
            return halted(was, after.map(|status| *status.state()));
        }
        let Some(after) = after else {
            return Self::Gone;
        };
        let state = *after.state();
        if action.in_transition(state) {
            return unsettled(state);
        }
        match action {
            Action::Pause => paused(was, state),
            Action::Resume => resumed(was, state),
            Action::Up | Action::Start | Action::Restart | Action::Run => {
                launched(action, before, after, saw_transition)
            },
            Action::Down | Action::Stop | Action::Kill | Action::Remove => halted(was, Some(state)),
        }
    }

    /// Whether the process did not end up as the action asked, which makes
    /// the command fail.
    pub fn is_failure(self) -> bool {
        match self {
            Self::Crashed
            | Self::Unsupervised
            | Self::StillStarting
            | Self::NotStarted
            | Self::StillStopping
            | Self::StillRunning
            | Self::Unchanged
            | Self::Refused
            | Self::Gone => true,
            Self::Started
            | Self::Restarted
            | Self::AlreadyRunning
            | Self::Exited
            | Self::Stopped
            | Self::WasNotRunning
            | Self::Paused
            | Self::AlreadyPaused
            | Self::Resumed => false,
        }
    }

    /// Whether the process was still on its way when the wait ended.
    pub fn is_unsettled(self) -> bool {
        match self {
            Self::StillStarting | Self::StillStopping => true,
            Self::Started
            | Self::Restarted
            | Self::AlreadyRunning
            | Self::Exited
            | Self::Crashed
            | Self::Unsupervised
            | Self::NotStarted
            | Self::Stopped
            | Self::WasNotRunning
            | Self::StillRunning
            | Self::Paused
            | Self::AlreadyPaused
            | Self::Resumed
            | Self::Unchanged
            | Self::Refused
            | Self::Gone => false,
        }
    }
}

/// The outcome of a process still on its way in `state` when the wait ended.
fn unsettled(state: ProcessState) -> Outcome {
    match state {
        ProcessState::Stopping => Outcome::StillStopping,
        ProcessState::Pending
        | ProcessState::Starting
        | ProcessState::Running
        | ProcessState::Paused
        | ProcessState::Exited
        | ProcessState::Crashed
        | ProcessState::Restarting
        | ProcessState::Unsupervised => Outcome::StillStarting,
    }
}

/// The outcome of a stop, kill, removal or down for a process that was
/// `was` and is now `now`, or no longer listed.
fn halted(was: Option<ProcessState>, now: Option<ProcessState>) -> Outcome {
    if !was.is_some_and(ProcessState::may_be_running) {
        return Outcome::WasNotRunning;
    }
    let Some(state) = now else {
        return Outcome::Stopped;
    };
    match state {
        ProcessState::Starting | ProcessState::Stopping => Outcome::StillStopping,
        ProcessState::Running
        | ProcessState::Paused
        | ProcessState::Restarting
        | ProcessState::Unsupervised => Outcome::StillRunning,
        ProcessState::Pending | ProcessState::Exited | ProcessState::Crashed => Outcome::Stopped,
    }
}

/// The outcome of a pause for a process that was `was` and is now `state`.
fn paused(was: Option<ProcessState>, state: ProcessState) -> Outcome {
    match state {
        ProcessState::Paused if was == Some(ProcessState::Paused) => Outcome::AlreadyPaused,
        ProcessState::Paused => Outcome::Paused,
        ProcessState::Pending
        | ProcessState::Starting
        | ProcessState::Running
        | ProcessState::Stopping
        | ProcessState::Exited
        | ProcessState::Crashed
        | ProcessState::Restarting
        | ProcessState::Unsupervised => Outcome::Unchanged,
    }
}

/// The outcome of a resume for a process that was `was` and is now `state`.
fn resumed(was: Option<ProcessState>, state: ProcessState) -> Outcome {
    match state {
        ProcessState::Running if was == Some(ProcessState::Running) => Outcome::AlreadyRunning,
        ProcessState::Running => Outcome::Resumed,
        ProcessState::Pending
        | ProcessState::Starting
        | ProcessState::Paused
        | ProcessState::Stopping
        | ProcessState::Exited
        | ProcessState::Crashed
        | ProcessState::Restarting
        | ProcessState::Unsupervised => Outcome::Unchanged,
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
    let restart = action == Action::Restart;
    match after.state() {
        ProcessState::Running | ProcessState::Paused if same_child && restart => {
            Outcome::StillRunning
        },
        ProcessState::Running | ProcessState::Paused if same_child => Outcome::AlreadyRunning,
        ProcessState::Running | ProcessState::Paused if restart => Outcome::Restarted,
        ProcessState::Running | ProcessState::Paused => Outcome::Started,
        ProcessState::Unsupervised => Outcome::Unsupervised,
        ProcessState::Pending
        | ProcessState::Starting
        | ProcessState::Stopping
        | ProcessState::Exited
        | ProcessState::Crashed
        | ProcessState::Restarting
            if !changed =>
        {
            Outcome::NotStarted
        },
        ProcessState::Exited => Outcome::Exited,
        ProcessState::Crashed | ProcessState::Restarting => Outcome::Crashed,
        ProcessState::Pending | ProcessState::Starting | ProcessState::Stopping => {
            Outcome::NotStarted
        },
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::domain::value::ProcessName;

    /// `api` listed in `state` with `pid` after `restarts` restarts.
    fn status(state: ProcessState, pid: Option<u32>, restarts: u32) -> ProcessStatus {
        ProcessStatus::builder()
            .name(ProcessName::try_new("api").unwrap())
            .state(state)
            .pid(pid)
            .uptime(pid.map(|_| Duration::from_secs(1)))
            .restarts(restarts)
            .usage(None)
            .attached(0)
            .last_exit(None)
            .stop_grace(Duration::from_secs(5))
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
        assert_eq!(
            Outcome::settle(Action::Start, Some(&stopped), None, true),
            Outcome::Gone
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
            Outcome::settle(Action::Remove, Some(&running), None, true),
            Outcome::Stopped,
            "a removed process's row retires once it stopped"
        );
        assert_eq!(
            Outcome::settle(Action::Remove, Some(&exited), None, false),
            Outcome::WasNotRunning
        );
        assert!(Outcome::StillStopping.is_failure());
        assert!(Outcome::StillRunning.is_failure());
    }

    /// Stopping or killing a crashed process stops nothing and fails
    /// nothing: it was not running.
    #[test]
    fn a_crashed_process_was_not_running() {
        let crashed = status(ProcessState::Crashed, None, 0);

        for action in [Action::Stop, Action::Kill] {
            let outcome = Outcome::settle(action, Some(&crashed), Some(&crashed), false);
            assert_eq!(outcome, Outcome::WasNotRunning, "{action}");
            assert!(!outcome.is_failure());
        }
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
            Outcome::settle(Action::Pause, Some(&paused), Some(&paused), false),
            Outcome::AlreadyPaused
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
