use getset::Getters;
use muster_domain::value::OutputCursor;
use serde::Serialize;
use typed_builder::TypedBuilder;

/// The line a wait matched, as `--json` shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct MatchView {
    /// The line, escapes removed.
    line: String,
    /// Where it began in the output.
    at: OutputCursor,
}
