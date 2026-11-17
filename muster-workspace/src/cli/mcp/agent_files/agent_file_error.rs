use std::{io, path::PathBuf};

use thiserror::Error;

use crate::cli::mcp::mcp_agent::McpAgent;

/// Why muster could not find, read or write one of an agent's config files,
/// or a repository's exclude file: what any of those files can fail with,
/// whatever muster wanted of it.
#[derive(Debug, Error)]
pub enum AgentFileError {
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
    /// The file is not JSON.
    #[error("{} is not valid JSON: {message}", path.display())]
    Malformed {
        /// The file.
        path: PathBuf,
        /// Where and why parsing stopped.
        message: String,
    },
    /// This user may not write the file or its folder, so muster leaves it
    /// as its owner set it.
    #[error("{} is not writable by you", path.display())]
    ReadOnly {
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    /// A file failure says what happened to that file and nothing of hooks,
    /// as it may be a servers file or an exclude file.
    #[test]
    fn a_file_failure_names_the_file_alone() {
        let path = Path::new("/home/me/.claude.json").to_path_buf();
        let failures = [
            AgentFileError::Read {
                path: path.clone(),
                source: io::Error::other("busy"),
            },
            AgentFileError::Malformed {
                path: path.clone(),
                message: "EOF".to_owned(),
            },
            AgentFileError::ReadOnly { path: path.clone() },
            AgentFileError::Write {
                path,
                source: io::Error::other("full"),
            },
        ];
        for failure in failures {
            let said = failure.to_string();
            assert!(said.contains("/home/me/.claude.json"), "{said}");
            assert!(!said.to_lowercase().contains("hook"), "{said}");
        }
    }
}
