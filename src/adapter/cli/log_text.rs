use std::path::PathBuf;

use getset::Getters;
use serde::Serialize;
use typed_builder::TypedBuilder;

use crate::domain::value::ProcessName;

/// What `muster logs` shows of one process: the `--json` output of `logs`,
/// and of each log line `logs -f` prints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct LogText {
    /// The project's folder.
    project: PathBuf,
    /// The process.
    name: ProcessName,
    /// The log file the text was read from, or none for the live screen.
    log: Option<PathBuf>,
    /// The text, with any invalid UTF-8 replaced.
    text: String,
}
