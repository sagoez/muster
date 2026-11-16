use crate::{config::Registering, value::ProjectRoot};

/// Where a shared write, such as a note, goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WritePlace {
    /// A registered project; the write fails when it is gone.
    Project(ProjectRoot),
    /// A folder no project covered: in the write's own transaction it goes
    /// to a project that has come to contain the folder, or registers the
    /// repository root.
    Unregistered(Registering),
}

impl From<ProjectRoot> for WritePlace {
    /// The registered project at `root`.
    fn from(root: ProjectRoot) -> Self {
        Self::Project(root)
    }
}
