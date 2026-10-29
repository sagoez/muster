use getset::Getters;
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::agent_wait_view::AgentWaitView;
use crate::{
    adapter::client::{ActionReport, NoticeView, Outcome, ProcessOutcome, ProjectIdentityView},
    domain::value::ProcessName,
};

/// What `stack_up` started, left alone and waited for.
#[derive(Debug, Clone, PartialEq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct UpView {
    /// The project.
    project: ProjectIdentityView,
    /// The processes it started.
    started: Vec<ProcessName>,
    /// The processes that were running already.
    already_running: Vec<ProcessName>,
    /// The processes the service left alone, each with why.
    refused: Vec<ProcessOutcome>,
    /// The processes that crashed, exited or had not settled when the call
    /// stopped waiting.
    others: Vec<ProcessOutcome>,
    /// What supervision reported about them meanwhile.
    notices: Vec<NoticeView>,
    /// Each wait, when the call waited.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    #[builder(default)]
    waits: Vec<AgentWaitView>,
}

impl UpView {
    /// The view with `waits`, one per process the call waited on.
    pub fn with_waits(mut self, waits: Vec<AgentWaitView>) -> Self {
        self.waits = waits;
        self
    }
}

impl From<&ActionReport> for UpView {
    fn from(report: &ActionReport) -> Self {
        let named = |outcome: Outcome| {
            report
                .processes()
                .iter()
                .filter(|process| process.outcome() == outcome)
                .map(|process| process.name().clone())
                .collect()
        };
        let (refused, others): (Vec<ProcessOutcome>, Vec<ProcessOutcome>) = report
            .processes()
            .iter()
            .filter(|process| {
                !matches!(
                    process.outcome(),
                    Outcome::Started | Outcome::AlreadyRunning
                )
            })
            .cloned()
            .partition(|process| process.outcome() == Outcome::Refused);
        Self {
            project: report.project().clone(),
            started: named(Outcome::Started),
            already_running: named(Outcome::AlreadyRunning),
            refused,
            others,
            notices: report.notices().clone(),
            waits: Vec::new(),
        }
    }
}
