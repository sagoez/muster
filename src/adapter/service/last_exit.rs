use std::time::SystemTime;

use getset::{CopyGetters, Getters};
use serde::{Deserialize, Serialize};
use strum::Display;
use typed_builder::TypedBuilder;

use super::wait_outcome::WaitStatus;
use crate::domain::pty::{ExitCause, ExitOutcome};

/// Why a process's run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display, Serialize, Deserialize)]
#[strum(serialize_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
pub enum ExitReason {
    /// It exited with status 0 on its own.
    Exited,
    /// It failed on its own.
    Crashed,
    /// A signal muster did not send ended it.
    Killed,
    /// It was stopped or restarted on request.
    Stopped,
    /// It could not be launched.
    FailedToStart,
}

impl ExitReason {
    /// Why a run ended with `outcome`, stopped on request when `requested`.
    pub fn of(outcome: &ExitOutcome, requested: bool) -> Self {
        match outcome {
            _ if requested => Self::Stopped,
            ExitOutcome::Succeeded => Self::Exited,
            ExitOutcome::Failed(ExitCause::Signal(_)) => Self::Killed,
            ExitOutcome::Failed(_) => Self::Crashed,
        }
    }

    /// How a wait on the run that ended this way ends.
    pub fn wait_status(self) -> WaitStatus {
        match self {
            Self::Exited => WaitStatus::Exited,
            Self::Crashed | Self::Killed | Self::FailedToStart => WaitStatus::Crashed,
            Self::Stopped => WaitStatus::Stopped,
        }
    }
}

/// How a process's most recent run ended.
#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, CopyGetters, TypedBuilder,
)]
#[serde(rename_all = "kebab-case")]
pub struct LastExit {
    /// Its exit status or signal.
    #[getset(get = "pub")]
    #[serde(with = "crate::adapter::wire::form")]
    outcome: ExitOutcome,
    /// Why it ended.
    #[getset(get_copy = "pub")]
    reason: ExitReason,
    /// When it ended, when this service saw it end.
    #[getset(get = "pub")]
    #[serde(with = "humantime_serde")]
    #[builder(default)]
    at: Option<SystemTime>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::pty::SignalName;

    #[test]
    fn a_request_outranks_how_the_run_ended() {
        let killed =
            ExitOutcome::Failed(ExitCause::Signal(SignalName::try_new("sigkill").unwrap()));

        assert_eq!(ExitReason::of(&killed, true), ExitReason::Stopped);
        assert_eq!(ExitReason::of(&killed, false), ExitReason::Killed);
        assert_eq!(
            ExitReason::of(&ExitOutcome::Succeeded, false),
            ExitReason::Exited
        );
        assert_eq!(
            ExitReason::of(&ExitOutcome::Failed(ExitCause::Status(1)), false),
            ExitReason::Crashed
        );
        assert_eq!(ExitReason::FailedToStart.to_string(), "failed-to-start");
    }
}
