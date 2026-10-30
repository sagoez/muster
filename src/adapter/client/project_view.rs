use getset::Getters;
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::{process_view::ProcessView, project_identity_view::ProjectIdentityView};
use crate::adapter::service::ProjectStatus;

/// One project's processes, as `muster ps --json` lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ProjectView {
    /// The project.
    project: ProjectIdentityView,
    /// Its processes, in the order they were added.
    processes: Vec<ProcessView>,
}

impl From<&ProjectStatus> for ProjectView {
    fn from(status: &ProjectStatus) -> Self {
        Self {
            project: ProjectIdentityView::from(status.project()),
            processes: status.processes().iter().map(ProcessView::from).collect(),
        }
    }
}
