use std::fmt;

use super::{exit_cause::ExitCause, size::PtySize};
use crate::domain::value::OutputCursor;

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

/// An event emitted by a running process, delivered to the runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessOutput {
    /// A chunk of raw bytes read from the process's PTY.
    Chunk(Vec<u8>),
    /// A chunk whose first byte sits at `at` in the process's logged output,
    /// so a receiver that missed chunks sees the gap.
    Placed {
        /// Where `bytes` start in the output stream.
        at: OutputCursor,
        /// The bytes.
        bytes: Vec<u8>,
    },
    /// Bytes that rebuild the process's screen in a fresh terminal of `size`:
    /// a repaint, not output the process wrote.
    Repaint {
        /// The bytes that rebuild the screen.
        bytes: Vec<u8>,
        /// The size the screen was rendered at.
        size: PtySize,
    },
    /// The process exited with the given outcome.
    Exited(ExitOutcome),
    /// The stream carrying the process's output could not be read any further,
    /// for the given reason. The process itself may still be running.
    Unreadable(String),
    /// Whatever runs the process recovered from a fault the user should know
    /// about, described.
    Warning(String),
    /// The writable clients now attached to the process's terminal.
    Attached(u32),
}
