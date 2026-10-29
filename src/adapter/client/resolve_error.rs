use std::path::PathBuf;

use thiserror::Error;

use crate::domain::{config::ConfigError, value::ProjectName};

/// Why the project a command or tool call names could not be found. Each
/// driver words it for its caller.
#[derive(Debug, Error)]
pub enum ResolveError {
    /// No registered project has the name given.
    #[error("no muster project is named '{name}'")]
    UnknownProject {
        /// The name as typed.
        name: String,
        /// The registered projects' names.
        valid: Vec<ProjectName>,
    },
    /// No registered project holds the folder.
    #[error("no muster project in {}", dir.display())]
    NotAProject {
        /// The folder the project was looked for from.
        dir: PathBuf,
    },
    /// Stored state could not be read, or the folder is not usable.
    #[error(transparent)]
    Config(#[from] ConfigError),
}
