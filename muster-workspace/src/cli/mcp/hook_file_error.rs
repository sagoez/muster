use std::{io, path::PathBuf};

use thiserror::Error;

use crate::cli::mcp::{hook_file_part::HookFilePart, json_shape::JsonShape, mcp_agent::McpAgent};

/// Why muster could not add its session-start hook to an agent's file, take
/// it out, or read the agent's config. muster then changes nothing.
#[derive(Debug, Error)]
pub enum HookFileError {
    /// No home folder to find the agent's own config folder in.
    #[error("no home folder to find {0}'s config in")]
    NoHome(McpAgent),
    /// The file could not be read.
    #[error("could not read {}: {source}", path.display())]
    Read {
        /// The file.
        path: PathBuf,
        /// What failed.
        #[source]
        source: io::Error,
    },
    /// The file is not JSON; muster does not edit what it cannot read.
    #[error("{} is not valid JSON, so muster changed nothing: {message}", path.display())]
    Malformed {
        /// The file.
        path: PathBuf,
        /// Where and why parsing stopped.
        message: String,
    },
    /// A part of the file the hook goes in is not what hooks files hold.
    #[error("{} holds {part} that is not {shape}, so muster changed nothing", path.display())]
    Unexpected {
        /// The file.
        path: PathBuf,
        /// The part.
        part: HookFilePart,
        /// What it should have been.
        shape: JsonShape,
    },
    /// The file is read-only, so muster leaves it as its owner set it.
    #[error("{} is read-only, so muster changed nothing", path.display())]
    ReadOnly {
        /// The file.
        path: PathBuf,
    },
    /// Something else kept changing the file while muster edited it.
    #[error("{} changed while muster edited it; run the command again", path.display())]
    Changed {
        /// The file.
        path: PathBuf,
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
