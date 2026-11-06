use getset::Getters;
use muster_domain::project::Project;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::{occupancy_status::OccupancyStatus, process_status::ProcessStatus};

/// A point-in-time view of one project's supervised processes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
#[serde(rename_all = "kebab-case")]
pub struct ProjectStatus {
    /// The project the processes belong to.
    #[serde(with = "muster_protocol::wire::form")]
    project: Project,
    /// Every process, in stack order.
    processes: Vec<ProcessStatus>,
    /// The shells in it and its idle stop.
    occupancy: OccupancyStatus,
}
