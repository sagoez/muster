use std::path::PathBuf;

use getset::Getters;
use muster_domain::{project::Project, value::ProjectName};
use serde::Serialize;
use typed_builder::TypedBuilder;

/// One registered project as `project_list` lists it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ListedProject {
    /// Its name, which a tool's `project` argument takes.
    name: ProjectName,
    /// Its folder.
    root: PathBuf,
    /// Whether it is the project a call naming none acts on.
    current: bool,
}

impl ListedProject {
    /// `project` listed, current when its root is `current`'s.
    pub fn of(project: &Project, current: Option<&Project>) -> Self {
        Self {
            name: project.name().clone(),
            root: project.root().as_ref().clone(),
            current: current.is_some_and(|current| current.root() == project.root()),
        }
    }
}
