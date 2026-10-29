use std::time::SystemTime;

use getset::Getters;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::{notice_level::NoticeLevel, printable::printable};
use crate::domain::value::{ProcessName, ProjectRoot};

/// One thing supervision has to tell the user, such as a failed start or a
/// forced stop, kept in place of a transient on-screen message.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
#[serde(rename_all = "kebab-case")]
pub struct Notice {
    /// When supervision raised it.
    #[serde(with = "humantime_serde")]
    at: SystemTime,
    /// The project it concerns, or none for one about the whole service, such
    /// as a holder record it could not read.
    project: Option<ProjectRoot>,
    /// The process it concerns, when it concerns one.
    process: Option<ProcessName>,
    /// How serious it is.
    level: NoticeLevel,
    /// What happened, for a person to read.
    message: String,
}

impl Notice {
    /// This notice with its message made [`printable`], since it may carry
    /// text a process or a config produced.
    pub fn printable(self) -> Self {
        Self {
            message: printable(&self.message),
            ..self
        }
    }
}
