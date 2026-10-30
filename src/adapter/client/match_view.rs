use getset::Getters;
use serde::Serialize;
use typed_builder::TypedBuilder;

use crate::domain::value::OutputCursor;

/// The line a wait matched, as `--json` shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct MatchView {
    /// The line, escapes removed.
    line: String,
    /// Where it began in the output.
    at: OutputCursor,
}
