use getset::Getters;
use serde::Serialize;
use typed_builder::TypedBuilder;

use crate::adapter::{
    client::WaitView,
    service::{WaitOutcome, WaitStatus},
};

/// What a wait that ran out of time tells the agent to do next.
const CALL_AGAIN: &str = "call process_wait again with since=";

/// A wait's outcome as an agent reads it: the shared view, plus how to
/// continue a wait that ran out of time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct AgentWaitView {
    /// How the wait ended.
    #[serde(flatten)]
    view: WaitView,
    /// What to do about a wait that ran out of time.
    #[serde(skip_serializing_if = "Option::is_none")]
    hint: Option<String>,
}

impl From<&WaitOutcome> for AgentWaitView {
    fn from(outcome: &WaitOutcome) -> Self {
        Self {
            view: WaitView::from(outcome),
            hint: (outcome.status() == WaitStatus::Waiting)
                .then(|| format!("{CALL_AGAIN}{}", outcome.cursor())),
        }
    }
}
