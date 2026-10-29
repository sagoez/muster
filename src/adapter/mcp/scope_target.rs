use super::tool_error::ToolError;
use crate::domain::{project::Project, value::ProjectRoot};

/// The project one tool call acts on: a registered one, or a folder no
/// project holds, which only `process_add` accepts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScopeTarget {
    /// A registered project.
    Registered(Project),
    /// A folder no registered project is rooted at or contains.
    Unregistered(ProjectRoot),
}

impl ScopeTarget {
    /// The folder the call acts on.
    pub fn root(&self) -> &ProjectRoot {
        match self {
            Self::Registered(project) => project.root(),
            Self::Unregistered(root) => root,
        }
    }

    /// The registered project, for every tool but `process_add`.
    ///
    /// # Errors
    /// Returns [`ToolError::NoProjectHere`] for a folder no project holds.
    pub fn registered(self) -> Result<Project, ToolError> {
        match self {
            Self::Registered(project) => Ok(project),
            Self::Unregistered(root) => Err(ToolError::NoProjectHere {
                dir: root.into_inner(),
            }),
        }
    }
}
