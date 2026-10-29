use std::path::PathBuf;

use getset::Getters;
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::process_view::ProcessView;
use crate::adapter::service::ProjectStatus;

/// One project's processes, as `muster ps --json` lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ProjectView {
    /// The project's config path.
    project: PathBuf,
    /// Its processes, in `muster.yml` order.
    processes: Vec<ProcessView>,
}

impl From<&ProjectStatus> for ProjectView {
    fn from(status: &ProjectStatus) -> Self {
        Self {
            project: status.project().as_ref().clone(),
            processes: status.processes().iter().map(ProcessView::from).collect(),
        }
    }
}
