use std::path::PathBuf;

use getset::Getters;
use typed_builder::TypedBuilder;

use super::log_text::LogText;
use crate::domain::value::{ProcessName, ProjectRoot};

/// What `muster logs` read of one process: its bytes exactly as written,
/// for a pipe to receive unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct LogRead {
    /// The process.
    name: ProcessName,
    /// The log file it was read from, or none for the live screen.
    log: Option<PathBuf>,
    /// What was read.
    bytes: Vec<u8>,
}

impl LogRead {
    /// The read as `--json` shows it, for the project at `root`, with any
    /// invalid UTF-8 replaced.
    pub fn text_in(&self, root: &ProjectRoot) -> LogText {
        LogText::builder()
            .project(root.as_ref().clone())
            .name(self.name.clone())
            .log(self.log.clone())
            .text(String::from_utf8_lossy(&self.bytes).into_owned())
            .build()
    }
}
