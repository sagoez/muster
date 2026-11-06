use getset::Getters;
use muster_domain::value::OutputCursor;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

/// The output line that satisfied a wait.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
#[serde(rename_all = "kebab-case")]
pub struct MatchedLine {
    /// The line, escapes removed.
    line: String,
    /// Where it began in the output.
    at: OutputCursor,
}
