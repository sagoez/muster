use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::{
    condition_arg::ConditionArg,
    match_view::MatchView,
    pending::{WaitOutcome, WaitStatus},
};
use crate::domain::value::ProcessName;

/// What `waiting` tells the agent to do next.
const CALL_AGAIN: &str = "call process_wait again with since=";

/// A wait's outcome as an agent reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct WaitView {
    /// The process waited on.
    #[getset(get = "pub")]
    process: ProcessName,
    /// How the wait ended.
    #[getset(get_copy = "pub")]
    status: WaitStatus,
    /// The condition, as process_wait takes it.
    #[getset(get = "pub")]
    until: ConditionArg,
    /// How long it waited.
    #[getset(get_copy = "pub")]
    elapsed_ms: u128,
    /// Pass as since to continue after what was examined.
    #[getset(get = "pub")]
    cursor: String,
    /// The line that matched a log condition.
    #[serde(rename = "match", skip_serializing_if = "Option::is_none")]
    #[getset(get = "pub")]
    matched: Option<MatchView>,
    /// The run's last lines, for a wait that did not match.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[getset(get = "pub")]
    tail: Vec<String>,
    /// Where scanning began, when since was older than the service holds.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[getset(get = "pub")]
    scanned_from: Option<String>,
    /// What to do about a wait that ran out of time.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[getset(get = "pub")]
    hint: Option<String>,
}

impl From<&WaitOutcome> for WaitView {
    fn from(outcome: &WaitOutcome) -> Self {
        let cursor = outcome.cursor().to_string();
        Self {
            process: outcome.process().clone(),
            status: outcome.status(),
            until: ConditionArg::from(outcome.until()),
            elapsed_ms: outcome.elapsed().as_millis(),
            matched: outcome.matched().as_ref().map(MatchView::from),
            tail: if outcome.status() == WaitStatus::Matched {
                Vec::new()
            } else {
                outcome.tail().clone()
            },
            scanned_from: outcome.scanned_from().map(|cursor| cursor.to_string()),
            hint: (outcome.status() == WaitStatus::Waiting)
                .then(|| format!("{CALL_AGAIN}{cursor}")),
            cursor,
        }
    }
}
