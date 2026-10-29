use std::path::PathBuf;

use getset::Getters;
use serde::Serialize;
use typed_builder::TypedBuilder;

use crate::domain::{project::Project, value::ProjectName};

/// A project as `--json` names it: `{"name": "shop", "root": "/home/me/shop"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ProjectIdentityView {
    /// Its name.
    name: ProjectName,
    /// Its folder.
    root: PathBuf,
}

impl From<&Project> for ProjectIdentityView {
    fn from(project: &Project) -> Self {
        Self {
            name: project.name().clone(),
            root: project.root().as_ref().clone(),
        }
    }
}
