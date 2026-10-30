use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::project::Project;

/// The project a write landed in, and whether that write registered it.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Registration {
    /// The project written to.
    project: Project,
    /// Whether the folder became a project with this write.
    newly_registered: bool,
}
