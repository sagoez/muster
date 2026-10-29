use std::path::PathBuf;

use getset::Getters;
use serde::Serialize;
use typed_builder::TypedBuilder;

use crate::{
    adapter::service::{Notice, NoticeLevel},
    domain::value::ProcessName,
};

/// One notice as `--json` shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct NoticeView {
    /// When it was raised, as RFC 3339 UTC to the second.
    at: String,
    /// The config path of the project it concerns, or none for the service.
    project: Option<PathBuf>,
    /// The process it concerns.
    process: Option<ProcessName>,
    /// How serious it is.
    level: NoticeLevel,
    /// What happened.
    message: String,
}

impl From<&Notice> for NoticeView {
    fn from(notice: &Notice) -> Self {
        Self {
            at: humantime::format_rfc3339_seconds(*notice.at()).to_string(),
            project: notice
                .project()
                .as_ref()
                .map(|project| project.as_ref().clone()),
            process: notice.process().clone(),
            level: *notice.level(),
            message: notice.message().clone(),
        }
    }
}
