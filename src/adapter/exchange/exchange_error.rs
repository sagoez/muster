use thiserror::Error;

use super::{EntryError, STACK_FILE_VERSION};
use crate::domain::value::ProcessName;

/// Why a stack file cannot be read or written.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ExchangeError {
    /// The text is not a stack file at all: not YAML, not a mapping, or
    /// missing or extra top-level keys.
    #[error("not a muster stack file: {reason}")]
    Malformed {
        /// What is wrong.
        reason: String,
    },
    /// The file declares a version this muster does not read.
    #[error(
        "stack file version {found} is not supported; this muster reads version {STACK_FILE_VERSION}"
    )]
    UnsupportedVersion {
        /// The version the file declares.
        found: u64,
    },
    /// One process entry is invalid.
    #[error("processes[{index}]{}: {source}", entry_label(name))]
    Entry {
        /// The entry's position in `processes`, from zero.
        index: usize,
        /// The entry's name, when it has a readable one.
        name: Option<String>,
        /// What is wrong with it.
        source: EntryError,
    },
    /// A process could not be exported as an entry an import accepts.
    #[error("cannot export {name}: {source}")]
    Unexportable {
        /// The process.
        name: ProcessName,
        /// Why an import would refuse its entry.
        source: EntryError,
    },
    /// The stack could not be rendered as YAML.
    #[error("could not render the stack file: {reason}")]
    Render {
        /// The serializer's account of the failure.
        reason: String,
    },
}

/// ` (name)` after an entry's index when it has a name, else nothing.
fn entry_label(name: &Option<String>) -> String {
    name.as_ref()
        .map(|name| format!(" ({name})"))
        .unwrap_or_default()
}
