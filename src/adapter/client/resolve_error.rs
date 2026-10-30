use std::path::PathBuf;

use thiserror::Error;

use super::refused_folder::RefusedFolder;
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
    /// The folder may not be registered as a project.
    #[error("{} is {why}, which cannot be a muster project", dir.display())]
    Unregistrable {
        /// The folder.
        dir: PathBuf,
        /// Why it is refused.
        why: RefusedFolder,
    },
    /// An absolute or `~` path led by a space, which taken as given would
    /// name a folder inside the current one.
    #[error(
        "the project {value:?} starts with a space before its path, so it would name a folder \
         inside the current one; drop the leading space"
    )]
    SpacedPath {
        /// The value as given.
        value: String,
    },
    /// Stored state could not be read, or the folder is not usable.
    #[error(transparent)]
    Config(#[from] ConfigError),
}
