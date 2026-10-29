use getset::Getters;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::{process_status::ProcessStatus, project_key::ProjectKey};

/// A point-in-time view of one project's supervised processes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
#[serde(rename_all = "kebab-case")]
pub struct ProjectStatus {
    /// The project the processes belong to.
    project: ProjectKey,
    /// Every process, in config section order.
    processes: Vec<ProcessStatus>,
}
