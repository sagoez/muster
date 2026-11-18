use getset::{CopyGetters, Getters};
use muster_client::ProjectIdentityView;
use muster_domain::project::Project;
use serde::Serialize;
use typed_builder::TypedBuilder;

/// One registered project as `project_list` lists it: its name and folder,
/// and whether it is the one a call naming none acts on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct ListedProject {
    /// Its name, which a tool's `project` argument takes, and its folder.
    #[serde(flatten)]
    #[getset(get = "pub")]
    project: ProjectIdentityView,
    /// Whether it is the project a call naming none acts on.
    #[getset(get_copy = "pub")]
    current: bool,
}

impl ListedProject {
    /// `project` listed, current when its root is `current`'s.
    pub fn of(project: &Project, current: Option<&Project>) -> Self {
        Self::builder()
            .project(ProjectIdentityView::from(project))
            .current(current.is_some_and(|current| current.root() == project.root()))
            .build()
    }
}
