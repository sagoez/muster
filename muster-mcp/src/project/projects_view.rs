use getset::Getters;
use serde::Serialize;
use typed_builder::TypedBuilder;

use crate::project::listed_project::ListedProject;

/// What `project_list` answers: every registered project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ProjectsView {
    /// The projects, each marked whether a call naming none acts on it.
    projects: Vec<ListedProject>,
}
