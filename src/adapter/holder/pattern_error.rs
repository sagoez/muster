use thiserror::Error;

/// A log pattern that cannot be compiled into a matcher.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PatternError {
    /// The regex does not parse, or compiles past the size limit.
    #[error("'{pattern}' is not a usable regex: {reason}")]
    InvalidRegex {
        /// The pattern as given.
        pattern: String,
        /// Why the regex engine refused it.
        reason: String,
    },
}
