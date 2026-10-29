use thiserror::Error;

use super::ProcessNameError;

/// Why a [`ProcessName`](super::ProcessName) was refused, in words a person
/// can act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum ProcessNameFault {
    /// Nothing was left once it was trimmed.
    #[error("a name must not be empty")]
    Empty,
    /// It holds a control character, which could steer a terminal.
    #[error("a name must not contain control characters")]
    ControlCharacter,
}

impl From<ProcessNameError> for ProcessNameFault {
    /// The fault `error` reports.
    fn from(error: ProcessNameError) -> Self {
        match error {
            ProcessNameError::NotEmptyViolated => Self::Empty,
            ProcessNameError::PredicateViolated => Self::ControlCharacter,
        }
    }
}
