use std::path::PathBuf;

use thiserror::Error;

use crate::cli::mcp::{
    agent_file_error::AgentFileError, hook_file_part::HookFilePart, json_shape::JsonShape,
};

/// Why muster could not add its session-start hook to an agent's hooks
/// file, or take it out. muster then changes nothing in that file.
#[derive(Debug, Error)]
pub enum HookFileError {
    /// The file itself could not be read, parsed or written.
    #[error(transparent)]
    File(#[from] AgentFileError),
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
    /// A key the hook goes under is in the file twice: the agents read the
    /// last, so muster cannot tell which runs.
    #[error("{} holds {part} twice, so muster changed nothing; keep one", path.display())]
    Duplicate {
        /// The file.
        path: PathBuf,
        /// The part held twice.
        part: HookFilePart,
    },
    /// Something else kept changing the file while muster edited it.
    #[error("{} changed while muster edited it; run the command again", path.display())]
    Changed {
        /// The file.
        path: PathBuf,
    },
}

impl HookFileError {
    /// Whether the file could be read but not parsed as a hooks file, so it
    /// holds no hook muster could find: not JSON, of another shape, or with
    /// a key twice.
    pub fn is_unparsable(&self) -> bool {
        matches!(
            self,
            Self::File(AgentFileError::Malformed { .. })
                | Self::Unexpected { .. }
                | Self::Duplicate { .. }
        )
    }
}
