use getset::Getters;
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::project_identity_view::ProjectIdentityView;
use crate::{
    adapter::service::{Notice, NoticeLevel},
    domain::{project::Project, value::ProcessName},
};

/// One notice as `--json` shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct NoticeView {
    /// When it was raised, as RFC 3339 UTC to the second.
    at: String,
    /// The project it concerns, or none for the service.
    project: Option<ProjectIdentityView>,
    /// The process it concerns.
    process: Option<ProcessName>,
    /// How serious it is.
    level: NoticeLevel,
    /// What happened.
    message: String,
}

impl NoticeView {
    /// `notice`, one the service held for `project`: it concerns that
    /// project or the service as a whole.
    pub fn for_project(notice: &Notice, project: &Project) -> Self {
        Self {
            at: humantime::format_rfc3339_seconds(*notice.at()).to_string(),
            project: notice
                .project()
                .as_ref()
                .map(|_| ProjectIdentityView::from(project)),
            process: notice.process().clone(),
            level: *notice.level(),
            message: notice.message().clone(),
        }
    }
}
