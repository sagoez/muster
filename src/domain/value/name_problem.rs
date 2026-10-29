use strum::Display;

use super::{process_name::ProcessNameError, process_ref::ProcessRefError};

/// Why a typed process name is refused, phrased to follow "it".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Display)]
pub enum NameProblem {
    /// Nothing but whitespace.
    #[strum(to_string = "is empty")]
    Empty,
    /// A control character, such as a newline or an escape.
    #[strum(to_string = "holds a control character")]
    ControlCharacter,
}

impl From<ProcessNameError> for NameProblem {
    fn from(error: ProcessNameError) -> Self {
        match error {
            ProcessNameError::NotEmptyViolated => Self::Empty,
            ProcessNameError::PredicateViolated => Self::ControlCharacter,
        }
    }
}

impl From<ProcessRefError> for NameProblem {
    fn from(error: ProcessRefError) -> Self {
        match error {
            ProcessRefError::NotEmptyViolated => Self::Empty,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::value::ProcessName;

    /// Each refused name says why.
    #[test]
    fn a_refused_name_says_why() {
        let problem = |raw: &str| NameProblem::from(ProcessName::try_new(raw).unwrap_err());
        assert_eq!(problem("  "), NameProblem::Empty);
        assert_eq!(problem("a\u{1b}]0;x\u{7}"), NameProblem::ControlCharacter);
        assert_eq!(NameProblem::Empty.to_string(), "is empty");
    }
}
