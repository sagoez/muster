use thiserror::Error;

use super::{name_problem::refused_name, process_name::ProcessName, process_ref::ProcessRef};

/// Why a [`ProcessRef`] names no single process. Each shows the reference
/// cut to the name cap, so a huge one is never echoed whole.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum UnresolvedProcess {
    /// No process has that name.
    #[error("no process is named '{}'", refused_name(.0.as_ref()))]
    Unknown(ProcessRef),
    /// Several processes have names that differ only in their Unicode
    /// encoding, and none matches byte for byte.
    #[error("'{}' matches more than one process", refused_name(.0.as_ref()))]
    Ambiguous(ProcessRef, Vec<ProcessName>),
}
