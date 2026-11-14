use std::path::PathBuf;

use thiserror::Error;

/// Why a folder cannot be a process's working directory.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WorkingDirError {
    /// The folder does not exist or is not a directory.
    #[error("{} is not a directory", .0.display())]
    NotADirectory(PathBuf),
    /// The folder's path is not UTF-8, so it cannot be stored.
    #[error("{} is not a UTF-8 path", .0.display())]
    Unrepresentable(PathBuf),
}
