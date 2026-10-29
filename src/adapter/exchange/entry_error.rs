use thiserror::Error;

/// Why one process entry of a stack file is refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EntryError {
    /// The entry does not have the file's shape: an unknown or missing key,
    /// or a value of the wrong type.
    #[error("{reason}")]
    Shape {
        /// The parser's account of what is wrong.
        reason: String,
    },
    /// `name` is blank.
    #[error("name must not be empty")]
    EmptyName,
    /// `command` is blank.
    #[error("command must not be empty")]
    EmptyCommand,
    /// `description` is present but blank.
    #[error("description must not be empty; leave the key out instead")]
    EmptyDescription,
    /// An environment variable's name cannot be exported.
    #[error(
        "'{name}' is not a usable environment variable name: it must be non-empty, contain no '=' or NUL, and not start with MUSTER_"
    )]
    InvalidEnvName {
        /// The name refused.
        name: String,
    },
    /// An environment variable's value contains a NUL byte.
    #[error("the value of {name} contains a NUL byte")]
    InvalidEnvValue {
        /// The variable whose value was refused.
        name: String,
    },
    /// An environment variable still holds the export's placeholder.
    #[error("{name} has no value; fill it in (or remove it) before importing")]
    MissingEnvValue {
        /// The variable left unfilled.
        name: String,
    },
    /// `ready` is not a usable readiness check.
    #[error("ready: {reason}")]
    InvalidReady {
        /// What is wrong with it.
        reason: String,
    },
    /// `stop.grace_period` is zero or longer than muster allows.
    #[error("stop.grace_period must be longer than zero and at most 1h")]
    GraceOutOfRange,
    /// An earlier entry has a name with the same key.
    #[error("a process with this name appears earlier in the file")]
    DuplicateName,
}
