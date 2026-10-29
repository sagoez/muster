use thiserror::Error;

use crate::domain::value::Pid;

/// Why a shell's files could not be written.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ShellRegistryError {
    /// A record or note file could not be written.
    #[error("cannot write the shell record for pid {pid}: {reason}")]
    Write {
        /// The shell whose file it was.
        pid: Pid,
        /// What the filesystem said.
        reason: String,
    },
}
