use std::fmt;

use getset::Getters;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use crate::domain::value::{ProcessName, ProjectName};

/// A supervised process listening on a port, named by its project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
#[serde(rename_all = "kebab-case")]
pub struct PortHolder {
    /// The project it belongs to.
    project: ProjectName,
    /// The process.
    process: ProcessName,
}

impl fmt::Display for PortHolder {
    /// Writes `project 'admin', process 'web'`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "project '{}', process '{}'",
            self.project, self.process
        )
    }
}
