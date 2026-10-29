use std::path::PathBuf;

use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::project_identity_view::ProjectIdentityView;
use crate::domain::value::{OutputCursor, ProcessName};

/// What `muster logs` shows of one process: the `--json` output of `logs`,
/// and of each log line `logs -f` prints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct LogText {
    /// The project.
    #[getset(get = "pub")]
    project: ProjectIdentityView,
    /// The process.
    #[getset(get = "pub")]
    process: ProcessName,
    /// The log file the text was read from, or none for the live screen.
    #[getset(get = "pub")]
    log: Option<PathBuf>,
    /// The text, with any invalid UTF-8 replaced.
    #[getset(get = "pub")]
    text: String,
    /// Just past the text in the process's output, for a log read or a line
    /// `logs -f` prints: the cursor to pass as `--since` to read what follows
    /// it. None for the screen.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    #[getset(get_copy = "pub")]
    cursor: Option<OutputCursor>,
    /// Bytes a rotation discarded before `logs -f` read them, on the line
    /// that reports the gap.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[builder(default)]
    #[getset(get_copy = "pub")]
    skipped_bytes: Option<u64>,
}
