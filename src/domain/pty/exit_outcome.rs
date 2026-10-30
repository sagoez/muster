use std::fmt;

use super::exit_cause::ExitCause;

/// What success says about how a process ended.
const SUCCEEDED_TEXT: &str = "exited with status 0";
/// The exit status of a successful process.
const SUCCESS_STATUS: u32 = 0;
/// Word before an exit status in a summary.
const STATUS_WORD: &str = "status";
/// Word before a signal name in a summary.
const SIGNAL_WORD: &str = "signal";

/// The exit disposition of a process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitOutcome {
    /// Exited with status zero.
    Succeeded,
    /// Exited with a non-zero status or was ended by a signal.
    Failed(ExitCause),
}

impl ExitOutcome {
    /// A failure whose cause was not observed.
    pub const FAILED_UNKNOWN: Self = Self::Failed(ExitCause::Unknown);

    /// Whether the process failed.
    pub fn is_failure(&self) -> bool {
        matches!(self, Self::Failed(_))
    }

    /// How it ended in a few words, `status 3` or `signal SIGKILL`, or none
    /// when its end was seen without either.
    pub fn summary(&self) -> Option<String> {
        match self {
            Self::Succeeded => Some(format!("{STATUS_WORD} {SUCCESS_STATUS}")),
            Self::Failed(ExitCause::Status(status)) => Some(format!("{STATUS_WORD} {status}")),
            Self::Failed(ExitCause::Signal(signal)) => Some(format!("{SIGNAL_WORD} {signal}")),
            Self::Failed(ExitCause::Unknown) => None,
        }
    }
}

impl fmt::Display for ExitOutcome {
    /// How the process ended, in words: `exited with status 1`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Succeeded => formatter.write_str(SUCCEEDED_TEXT),
            Self::Failed(cause) => cause.fmt(formatter),
        }
    }
}
