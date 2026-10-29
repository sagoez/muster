use serde::{Deserialize, Serialize};

use crate::domain::pty::{ExitCause, SignalName};

/// Why a process ended without success, as the holder and service carry it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub enum ExitCauseWire {
    /// It exited with this non-zero status.
    Status(u32),
    /// A signal ended it.
    Signal(SignalName),
    /// Its end was observed without a status.
    Unknown,
}

impl From<&ExitCause> for ExitCauseWire {
    fn from(cause: &ExitCause) -> Self {
        match cause {
            ExitCause::Status(status) => Self::Status(*status),
            ExitCause::Signal(signal) => Self::Signal(signal.clone()),
            ExitCause::Unknown => Self::Unknown,
        }
    }
}

impl From<ExitCauseWire> for ExitCause {
    fn from(wire: ExitCauseWire) -> Self {
        match wire {
            ExitCauseWire::Status(status) => Self::Status(status),
            ExitCauseWire::Signal(signal) => Self::Signal(signal),
            ExitCauseWire::Unknown => Self::Unknown,
        }
    }
}
