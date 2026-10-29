use std::path::PathBuf;

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
    /// `name` holds a control character.
    #[error("name must not contain control characters")]
    ControlCharacterName,
    /// `command` is blank.
    #[error("command must not be empty")]
    EmptyCommand,
    /// `working_dir` is present but empty.
    #[error("working_dir must not be empty; leave the key out to run in the project root")]
    EmptyWorkingDir,
    /// `working_dir` is absolute or climbs above the project root.
    #[error("working_dir {} must be a folder under the project root, given relative to it", dir.display())]
    WorkingDirOutsideRoot {
        /// The directory refused.
        dir: PathBuf,
    },
    /// `working_dir` lies under the project root as written, but a symlink
    /// on the way leads outside it.
    #[error(
        "working_dir {} resolves to {}, outside the project root",
        dir.display(),
        resolved.display()
    )]
    WorkingDirLeavesRoot {
        /// The directory as written.
        dir: PathBuf,
        /// Where it leads once symlinks are followed.
        resolved: PathBuf,
    },
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
