use thiserror::Error;

use super::{process_name::ProcessName, process_ref::ProcessRef};

/// Why a [`ProcessRef`] names no single process.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum UnresolvedProcess {
    /// No process has that name.
    #[error("no process is named '{0}'")]
    Unknown(ProcessRef),
    /// Several processes have names that differ only in their Unicode
    /// encoding, and none matches byte for byte.
    #[error("'{0}' matches more than one process")]
    Ambiguous(ProcessRef, Vec<ProcessName>),
}
