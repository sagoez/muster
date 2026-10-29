use std::path::Path;

use crate::domain::{
    config::ConfigError,
    port::ProjectStore,
    project::Project,
    value::{ProjectName, ProjectRoot},
};

/// A store holding a fixed set of projects, looked up as the database would.
pub(crate) struct FixedProjects(Vec<Project>);

impl FixedProjects {
    /// A store holding `projects`.
    pub(crate) fn new(projects: Vec<Project>) -> Self {
        Self(projects)
    }
}

impl ProjectStore for FixedProjects {
    fn projects(&self) -> Result<Vec<Project>, ConfigError> {
        Ok(self.0.clone())
    }

    fn project_at(&self, root: &ProjectRoot) -> Result<Option<Project>, ConfigError> {
        Ok(self
            .0
            .iter()
            .find(|project| project.root() == root)
            .cloned())
    }

    fn project_named(&self, name: &ProjectName) -> Result<Option<Project>, ConfigError> {
        Ok(self
            .0
            .iter()
            .find(|project| project.name() == name)
            .cloned())
    }

    fn project_containing(&self, dir: &Path) -> Result<Option<Project>, ConfigError> {
        Ok(self
            .0
            .iter()
            .filter(|project| project.root().contains(dir))
            .max_by_key(|project| project.root().as_ref().components().count())
            .cloned())
    }

    fn remove_project(&self, _root: &ProjectRoot) -> Result<bool, ConfigError> {
        Ok(false)
    }
}
