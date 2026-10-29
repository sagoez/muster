use std::time::SystemTime;

use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::value::ProcessName;

/// One agent edit as a project's edit log keeps it: when, which process, and
/// the edit's text as the user reads it.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct RecordedEdit {
    /// When the store recorded it.
    at: SystemTime,
    /// The process the edit concerns.
    process: ProcessName,
    /// The edit's text, which never shows an environment value.
    message: String,
}
