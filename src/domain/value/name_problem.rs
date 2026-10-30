use std::fmt::{self, Display, Formatter};

use super::{
    process_name::{PROCESS_NAME_MAX_CHARS, ProcessNameError},
    process_ref::ProcessRefError,
};

/// Marks a refused name cut short where a refusal shows it.
const ELIDED: &str = "...";

/// Why a process name is refused, phrased to follow the name or "it"; the CLI,
/// MCP and import all refuse a name with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameProblem {
    /// Nothing but whitespace.
    Empty,
    /// A control character, such as a newline or an escape, or a format
    /// character, such as a bidi override or a zero-width space.
    UnsafeCharacter,
}

impl Display for NameProblem {
    /// The reason.
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("is empty"),
            Self::UnsafeCharacter => f.write_str("holds a control or format character"),
        }
    }
}

impl From<ProcessNameError> for NameProblem {
    fn from(error: ProcessNameError) -> Self {
        match error {
            ProcessNameError::NotEmptyViolated => Self::Empty,
            ProcessNameError::PredicateViolated => Self::UnsafeCharacter,
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

/// `raw` as a refusal shows it: cut to [`PROCESS_NAME_MAX_CHARS`] characters
/// and marked when longer, so refusing a huge name never echoes it whole.
pub fn refused_name(raw: &str) -> String {
    match raw.char_indices().nth(PROCESS_NAME_MAX_CHARS) {
        Some((cut, _)) => format!("{}{ELIDED}", &raw[..cut]),
        None => raw.to_string(),
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
        assert_eq!(problem("a\u{1b}]0;x\u{7}"), NameProblem::UnsafeCharacter);
        assert_eq!(NameProblem::Empty.to_string(), "is empty");
    }

    /// A refusal shows a long name cut short, and a short one whole.
    #[test]
    fn a_refused_name_is_shown_bounded() {
        assert_eq!(refused_name("web"), "web");
        let shown = refused_name(&"\u{e9}".repeat(PROCESS_NAME_MAX_CHARS * 1000));
        assert_eq!(
            shown,
            format!("{}{ELIDED}", "\u{e9}".repeat(PROCESS_NAME_MAX_CHARS))
        );
    }
}
