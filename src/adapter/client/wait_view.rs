use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::condition_view::ConditionView;
use crate::{
    adapter::service::{WaitOutcome, WaitStatus},
    domain::value::{OutputCursor, ProcessName},
};

/// The line a wait matched, as `--json` shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct MatchView {
    /// The line, escapes removed.
    line: String,
    /// Where it began in the output.
    at: OutputCursor,
}

/// How a wait ended, as `--json` shows it: elapsed time in whole
/// milliseconds, the condition in words, cursors in their text form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct WaitView {
    /// The process waited on.
    #[getset(get = "pub")]
    process: ProcessName,
    /// How the wait ended.
    #[getset(get_copy = "pub")]
    status: WaitStatus,
    /// What was waited for, in the shape a wait condition is written.
    #[getset(get = "pub")]
    until: ConditionView,
    /// Whole milliseconds it waited.
    #[getset(get_copy = "pub")]
    elapsed_ms: u64,
    /// Where the output stood when it ended; output after it is new.
    #[getset(get_copy = "pub")]
    cursor: OutputCursor,
    /// The line that matched a log condition.
    #[serde(rename = "match", skip_serializing_if = "Option::is_none")]
    #[getset(get = "pub")]
    matched: Option<MatchView>,
    /// The run's last lines, when the wait did not match.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[getset(get = "pub")]
    tail: Vec<String>,
    /// Where scanning began, when the cursor asked for was older than the
    /// output still held.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[getset(get_copy = "pub")]
    scanned_from: Option<OutputCursor>,
}

impl From<&WaitOutcome> for WaitView {
    fn from(outcome: &WaitOutcome) -> Self {
        Self {
            process: outcome.process().clone(),
            status: outcome.status(),
            until: ConditionView::from(outcome.until()),
            elapsed_ms: u64::try_from(outcome.elapsed().as_millis()).unwrap_or(u64::MAX),
            cursor: *outcome.cursor(),
            matched: outcome.matched().as_ref().map(|matched| MatchView {
                line: matched.line().clone(),
                at: *matched.at(),
            }),
            tail: outcome.tail().clone(),
            scanned_from: *outcome.scanned_from(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::adapter::service::{MatchedLine, StateGoal, WaitCondition};

    #[test]
    fn reads_as_the_agent_facing_shape() {
        let outcome = WaitOutcome::builder()
            .process(ProcessName::try_new("api").unwrap())
            .status(WaitStatus::Matched)
            .until(WaitCondition::State(StateGoal::Running))
            .matched(Some(
                MatchedLine::builder()
                    .line("Listening on :3000".to_string())
                    .at(OutputCursor::new(18190))
                    .build(),
            ))
            .cursor(OutputCursor::new(18234))
            .elapsed(Duration::from_millis(2140))
            .build();

        insta::assert_snapshot!(serde_json::to_string_pretty(&WaitView::from(&outcome)).unwrap());
    }
}
