use std::path::PathBuf;

use thiserror::Error;

/// Why no hook script could be written for a path.
#[derive(Debug, Error)]
pub enum HookScriptError {
    /// The path is not UTF-8, so no shell script text can carry it.
    #[error("{} is not valid UTF-8, so the shell hook cannot name it", path.display())]
    NotUtf8 {
        /// The path that is not UTF-8.
        path: PathBuf,
    },
    /// The path holds a nul byte, which no shell word can carry.
    #[error("{} holds a nul byte, so the shell hook cannot name it", path.display())]
    Unquotable {
        /// The path holding the nul byte.
        path: PathBuf,
    },
}
