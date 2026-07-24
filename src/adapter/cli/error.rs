use thiserror::Error;

use crate::domain::config::ConfigError;

/// A failure while executing a CLI command.
#[derive(Debug, Error)]
pub enum CliError {
    /// No registered project matched the requested name.
    #[error("unknown project '{0}'")]
    UnknownProject(String),
    /// The derived or given process name was empty.
    #[error("'{0}' is not a valid process name")]
    InvalidName(String),
    /// The command was blank.
    #[error("the command is empty")]
    EmptyCommand,
    /// The command could not be reassembled into a shell string.
    #[error("the command cannot be represented as a shell command")]
    InvalidCommand,
    /// `muster run` is not available on this platform.
    #[error("muster run is only supported on Unix")]
    Unsupported,
    /// The workspace file could not be read or written.
    #[error(transparent)]
    Config(#[from] ConfigError),
    /// The command could not be executed.
    #[error("failed to run the command: {0}")]
    Exec(#[source] std::io::Error),
    /// The folder name could not become a project name.
    #[error("cannot derive a project name from '{0}'")]
    InvalidProjectFolder(std::path::PathBuf),
    /// The folder has no workspace file to register.
    #[error("{0} not found; run `muster init` there first")]
    MissingWorkspaceFile(std::path::PathBuf),
    /// No registered project matched, listing what exists.
    #[error("unknown project '{name}'; registered: {known}")]
    UnknownProjectAmong { name: String, known: String },
}
