use getset::Getters;
use muster_client::{DefinitionView, ProjectIdentityView};
use serde::Serialize;
use typed_builder::TypedBuilder;

/// What `process_add` answers: the project and the process as stored.
#[derive(Debug, Clone, PartialEq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct AddedView {
    /// The project the process was added to.
    project: ProjectIdentityView,
    /// The stored definition, its environment as variable names only.
    process: DefinitionView,
}
