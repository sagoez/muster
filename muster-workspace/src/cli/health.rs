use muster_domain::process::{ProcessState, Readiness};
use muster_protocol::service::{ExitReason, LastExit, ProcessStatus, WAIT_KIND_PROPERTY};
use strum::{Display, EnumIter, EnumProperty};

use super::row_kind::RowKind;

/// The property naming the [`RowKind`] a health is marked as, the one a
/// wait's ending is marked by.
const KIND_PROPERTY: &str = WAIT_KIND_PROPERTY;

/// How a process stands, as a person reads it: the one place a process's
/// state, readiness and last exit become a word and a mark. Declared in the
/// order `ps` counts them, problems first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Display, EnumIter, EnumProperty)]
#[strum(serialize_all = "kebab-case")]
pub enum Health {
    /// It ended with a failure: the human word for a crash.
    #[strum(props(kind = "fail"))]
    Failed,
    /// A port it needs is held, so it did not start.
    #[strum(props(kind = "fail"))]
    Blocked,
    /// It runs and is ready, or has no readiness check.
    #[strum(props(kind = "ok"))]
    Running,
    /// It is starting, or runs but has not passed its readiness check yet.
    #[strum(props(kind = "hint"))]
    Starting,
    /// It runs but did not pass its readiness check in time.
    #[strum(to_string = "not ready", props(kind = "hint"))]
    NotReady,
    /// It waits out a backoff before its policy starts it again.
    #[strum(props(kind = "hint"))]
    Restarting,
    /// It was asked to end and has not yet.
    #[strum(props(kind = "hint"))]
    Stopping,
    /// It may still run under a holder the service cannot reach.
    #[strum(props(kind = "hint"))]
    Unsupervised,
    /// It is suspended.
    #[strum(props(kind = "idle"))]
    Paused,
    /// It was stopped on request, while it ran or while its policy waited
    /// to start it again.
    #[strum(props(kind = "idle"))]
    Stopped,
    /// It ran and exited on its own with status 0.
    #[strum(props(kind = "idle"))]
    Exited,
    /// It has not been started.
    #[strum(props(kind = "idle"))]
    Pending,
}

impl Health {
    /// How a process in `state` stands, given its run's `ready` and how its
    /// last run ended.
    pub fn of(state: ProcessState, ready: Readiness, last_exit: Option<&LastExit>) -> Self {
        match state {
            ProcessState::Pending => Self::Pending,
            ProcessState::Starting => Self::Starting,
            ProcessState::Running => match ready {
                Readiness::Waiting => Self::Starting,
                Readiness::TimedOut => Self::NotReady,
                Readiness::None | Readiness::Ready => Self::Running,
            },
            ProcessState::Paused => Self::Paused,
            ProcessState::Stopping => Self::Stopping,
            // A failed run that ends up exited, not crashed, was stopped
            // during the backoff before its restart.
            ProcessState::Exited
                if last_exit.is_some_and(|exit| {
                    exit.reason() == ExitReason::Stopped || exit.outcome().is_failure()
                }) =>
            {
                Self::Stopped
            },
            ProcessState::Exited => Self::Exited,
            ProcessState::Crashed => Self::Failed,
            ProcessState::Restarting => Self::Restarting,
            ProcessState::Unsupervised => Self::Unsupervised,
            ProcessState::Blocked => Self::Blocked,
        }
    }

    /// How `process` stands.
    pub fn of_process(process: &ProcessStatus) -> Self {
        Self::of(
            *process.state(),
            *process.ready(),
            process.last_exit().as_ref(),
        )
    }

    /// How `process` stands by its state and last exit alone, readiness left
    /// to a column of its own, as the wide table and the chooser show it.
    pub fn of_lifecycle(process: &ProcessStatus) -> Self {
        Self::of(
            *process.state(),
            Readiness::None,
            process.last_exit().as_ref(),
        )
    }

    /// The kind of line, and so the mark, this health is shown with.
    pub fn kind(self) -> RowKind {
        RowKind::of_property(self.get_str(KIND_PROPERTY))
    }

    /// Whether it is idle in a way a start changes: stopped, exited or
    /// never started.
    pub fn is_down(self) -> bool {
        matches!(self, Self::Stopped | Self::Exited | Self::Pending)
    }
}

impl From<ProcessState> for Health {
    /// How a process in `state` stands when nothing else is known of it.
    fn from(state: ProcessState) -> Self {
        Self::of(state, Readiness::None, None)
    }
}

#[cfg(test)]
mod tests {
    use muster_domain::pty::{ExitCause, ExitOutcome};
    use strum::IntoEnumIterator;

    use super::*;

    /// Every health has a mark, and only failures are problems.
    #[test]
    fn every_health_is_marked() {
        for health in Health::iter() {
            assert_ne!(health.kind(), RowKind::Plain, "{health}");
            assert_eq!(
                health.kind() == RowKind::Fail,
                matches!(health, Health::Failed | Health::Blocked),
                "{health}"
            );
        }
    }

    /// A crash reads as failed, never crashed, and a run still waiting on
    /// its readiness check reads as starting.
    #[test]
    fn a_crash_reads_as_failed() {
        assert_eq!(
            Health::of(ProcessState::Crashed, Readiness::None, None).to_string(),
            "failed"
        );
        assert_eq!(
            Health::of(ProcessState::Running, Readiness::Waiting, None),
            Health::Starting
        );
        assert_eq!(
            Health::of(ProcessState::Running, Readiness::TimedOut, None).to_string(),
            "not ready"
        );
        assert!(Health::iter().all(|health| !health.to_string().contains("crash")));
    }

    /// A process stopped while its policy waited to restart it reads as
    /// stopped, though its last run failed; one that exited cleanly on its
    /// own reads as exited.
    #[test]
    fn a_stop_during_the_backoff_reads_as_stopped() {
        let ended = |outcome: ExitOutcome| {
            LastExit::builder()
                .reason(ExitReason::of(&outcome, false))
                .outcome(outcome)
                .build()
        };
        let crashed = ended(ExitOutcome::Failed(ExitCause::Status(28)));
        assert_eq!(
            Health::of(ProcessState::Exited, Readiness::None, Some(&crashed)),
            Health::Stopped
        );
        let clean = ended(ExitOutcome::Succeeded);
        assert_eq!(
            Health::of(ProcessState::Exited, Readiness::None, Some(&clean)),
            Health::Exited
        );
    }
}
