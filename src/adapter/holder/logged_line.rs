use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use crate::domain::value::OutputCursor;

/// One line read back from a process's log, stripped, and where it began.
#[derive(Debug, Clone, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub struct LoggedLine {
    /// Where the line began in the output.
    #[getset(get_copy = "pub")]
    at: OutputCursor,
    /// The line, escapes removed and carriage returns applied.
    #[getset(get = "pub")]
    text: String,
}
