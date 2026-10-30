use getset::Getters;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::process_status::ProcessStatus;
use crate::domain::project::Project;

/// A point-in-time view of one project's supervised processes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
#[serde(rename_all = "kebab-case")]
pub struct ProjectStatus {
    /// The project the processes belong to.
    #[serde(with = "crate::adapter::wire::form")]
    project: Project,
    /// Every process, in stack order.
    processes: Vec<ProcessStatus>,
}
