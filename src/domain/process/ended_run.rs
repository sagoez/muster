use std::time::SystemTime;

use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::{pty::ExitOutcome, value::ProcessName};

/// How a process's last run ended on its own, kept while it does not run so
/// a later service still shows it crashed or exited.
#[derive(Debug, Clone, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct EndedRun {
    /// The process.
    process: ProcessName,
    /// Its exit status or signal.
    outcome: ExitOutcome,
    /// When it ended.
    at: SystemTime,
}
