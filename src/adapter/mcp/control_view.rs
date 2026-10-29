use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use crate::{
    adapter::client::{
        ActionReport, NoticeView, Outcome, ProcessView, ProjectIdentityView, WaitEnd,
    },
    domain::value::ProcessName,
};

/// What a start, stop or restart of one process came to.
#[derive(Debug, Clone, PartialEq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct ControlView {
    /// The project.
    #[getset(get = "pub")]
    project: ProjectIdentityView,
    /// The process.
    #[getset(get = "pub")]
    process: Option<ProcessName>,
    /// What became of it.
    #[getset(get_copy = "pub")]
    outcome: Option<Outcome>,
    /// Why the service left it alone, for a refused outcome.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[getset(get = "pub")]
    reason: Option<String>,
    /// How stack_status lists it now.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[getset(get = "pub")]
    status: Option<ProcessView>,
    /// What supervision reported about it meanwhile.
    #[getset(get = "pub")]
    notices: Vec<NoticeView>,
    /// How the wait for it to settle ended.
    #[getset(get_copy = "pub")]
    wait_ended: WaitEnd,
}

impl From<&ActionReport> for ControlView {
    fn from(report: &ActionReport) -> Self {
        let process = report.processes().first();
        Self {
            project: report.project().clone(),
            process: process.map(|process| process.name().clone()),
            outcome: process.map(|process| process.outcome()),
            reason: process.and_then(|process| process.reason().clone()),
            status: process.and_then(|process| process.status().clone()),
            notices: report.notices().clone(),
            wait_ended: report.wait_ended(),
        }
    }
}
