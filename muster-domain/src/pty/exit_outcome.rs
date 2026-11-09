use std::fmt;

use super::exit_cause::ExitCause;

/// How a successful end reads.
const SUCCEEDED_TEXT: &str = "exit 0";

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
}

impl fmt::Display for ExitOutcome {
    /// How the process ended, as every surface says it: `exit 1`,
    /// `signal SIGKILL`, `no exit status`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Succeeded => formatter.write_str(SUCCEEDED_TEXT),
            Self::Failed(cause) => cause.fmt(formatter),
        }
    }
}
