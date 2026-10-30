use getset::Getters;
use typed_builder::TypedBuilder;

use crate::{adapter::client::ProjectTarget, domain::config::ProcessSpec};

/// A stack file read and checked whole, ready to plan against a project.
#[derive(Debug, Clone, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct StackRead {
    /// The project it goes into.
    target: ProjectTarget,
    /// The processes it defines.
    processes: Vec<ProcessSpec>,
}
