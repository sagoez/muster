use std::{io, path::PathBuf};

use thiserror::Error;

/// Why muster could not add its session-start hook to an agent's hooks
/// file, or take it out. The file is left as it was.
#[derive(Debug, Error)]
pub enum HookFileError {
    /// No home folder to find the agent's own config folder in.
    #[error("no home folder to find {0} in")]
    NoHome(&'static str),
    /// The file could not be read.
    #[error("could not read {}: {source}", path.display())]
    Read {
        /// The file.
        path: PathBuf,
        /// What failed.
        #[source]
        source: io::Error,
    },
    /// The file is not JSON; muster does not rewrite what it cannot read.
    #[error("{} is not valid JSON, so muster left it as it was: {source}", path.display())]
    Malformed {
        /// The file.
        path: PathBuf,
        /// Where parsing stopped.
        #[source]
        source: serde_json::Error,
    },
    /// A part of the file the hook goes in is not what hooks files hold.
    #[error("{} holds {what} that is not {expected}, so muster left it as it was", path.display())]
    Unexpected {
        /// The file.
        path: PathBuf,
        /// The part, as a person would find it.
        what: &'static str,
        /// What it should have been.
        expected: &'static str,
    },
    /// The file could not be written.
    #[error("could not write {}: {source}", path.display())]
    Write {
        /// The file.
        path: PathBuf,
        /// What failed.
        #[source]
        source: io::Error,
    },
}
