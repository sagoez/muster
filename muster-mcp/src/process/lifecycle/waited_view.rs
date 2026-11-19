use getset::{CopyGetters, Getters};
use muster_client::Outcome;
use muster_domain::value::ProcessName;
use muster_protocol::service::WaitOutcome;
use serde::Serialize;
use typed_builder::TypedBuilder;

use crate::process::lifecycle::agent_wait_view::AgentWaitView;

/// What a start or restart that waited answers: the process and how the
/// wait ended.
#[derive(Debug, Clone, PartialEq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct WaitedView {
    /// The process waited on.
    #[getset(get = "pub")]
    process: ProcessName,
    /// How the wait ended.
    #[getset(get = "pub")]
    wait: AgentWaitView,
    /// `already-running` when the service found it running or starting and
    /// started nothing.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[getset(get_copy = "pub")]
    outcome: Option<Outcome>,
}

impl From<&WaitOutcome> for WaitedView {
    fn from(outcome: &WaitOutcome) -> Self {
        Self::builder()
            .process(outcome.process().clone())
            .wait(AgentWaitView::from(outcome))
            .outcome(outcome.already_running().then_some(Outcome::AlreadyRunning))
            .build()
    }
}
