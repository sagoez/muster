use getset::Getters;
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::pending::MatchedLine;

/// The line a log condition matched, as an agent reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct MatchView {
    /// The line.
    line: String,
    /// Where it began.
    at: String,
}

impl From<&MatchedLine> for MatchView {
    fn from(matched: &MatchedLine) -> Self {
        Self {
            line: matched.line().clone(),
            at: matched.at().to_string(),
        }
    }
}
