use thiserror::Error;

use crate::domain::process::PATTERN_MAX_CHARS;

/// A log pattern that cannot be used to match lines.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PatternError {
    /// The text is blank or longer than a pattern may be.
    #[error("'{0}' is not a valid pattern; pass 1 to {max} characters", max = PATTERN_MAX_CHARS)]
    Malformed(String),
    /// The regex does not parse, or compiles past the size limit.
    #[error("'{pattern}' is not a usable regex: {reason}")]
    InvalidRegex {
        /// The pattern as given.
        pattern: String,
        /// Why the regex engine refused it.
        reason: String,
    },
}
