use getset::Getters;
use muster_client::ProjectIdentityView;
use serde::Serialize;
use typed_builder::TypedBuilder;

/// What `project_register` answers: the project the folder is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ProjectView {
    /// The project, as `{name, root}`.
    project: ProjectIdentityView,
}
