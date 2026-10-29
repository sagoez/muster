use std::path::Path;

use crate::domain::{
    config::ConfigError,
    port::ProjectStore,
    project::Project,
    value::{ProjectName, ProjectRoot},
};

/// A store holding a fixed set of projects.
pub(super) struct Projects(pub(super) Vec<Project>);

impl ProjectStore for Projects {
    fn projects(&self) -> Result<Vec<Project>, ConfigError> {
        Ok(self.0.clone())
    }

    fn project_at(&self, _root: &ProjectRoot) -> Result<Option<Project>, ConfigError> {
        Ok(None)
    }

    fn project_named(&self, _name: &ProjectName) -> Result<Option<Project>, ConfigError> {
        Ok(None)
    }

    fn project_containing(&self, _dir: &Path) -> Result<Option<Project>, ConfigError> {
        Ok(None)
    }

    fn remove_project(&self, _root: &ProjectRoot) -> Result<bool, ConfigError> {
        Ok(false)
    }
}
