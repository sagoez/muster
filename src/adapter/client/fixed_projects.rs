use std::path::Path;

use crate::domain::{
    config::{ConfigError, UnreadableProject},
    occupancy::RegisteredProject,
    port::ProjectStore,
    project::Project,
    value::{EpochNanos, ProjectName, ProjectRoot},
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

    fn unreadable_projects(&self) -> Result<Vec<UnreadableProject>, ConfigError> {
        Ok(Vec::new())
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

    /// Every fixed project was registered at the epoch.
    fn registered_project_containing(
        &self,
        dir: &Path,
    ) -> Result<Option<RegisteredProject>, ConfigError> {
        Ok(self
            .0
            .iter()
            .filter(|project| project.root().contains(dir))
            .max_by_key(|project| project.root().as_ref().components().count())
            .map(|project| {
                RegisteredProject::builder()
                    .project(project.clone())
                    .registered(Some(EpochNanos::new(0)))
                    .build()
            }))
    }

    fn remove_project(&self, _root: &ProjectRoot) -> Result<bool, ConfigError> {
        Ok(false)
    }

    fn remove_unreadable_project(&self, _stored: &Path) -> Result<bool, ConfigError> {
        Ok(false)
    }
}
