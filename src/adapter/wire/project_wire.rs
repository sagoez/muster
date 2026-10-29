use serde::{Deserialize, Serialize};

use super::WireForm;
use crate::domain::{
    project::Project,
    value::{ProjectName, ProjectRoot},
};

/// A registered project, as the service reports it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectWire {
    /// The folder the project is.
    root: ProjectRoot,
    /// Its display name.
    name: ProjectName,
}

impl WireForm for Project {
    type Wire = ProjectWire;

    fn to_wire(&self) -> ProjectWire {
        ProjectWire {
            root: self.root().clone(),
            name: self.name().clone(),
        }
    }

    fn from_wire(wire: ProjectWire) -> Result<Self, String> {
        Ok(Self::builder().root(wire.root).name(wire.name).build())
    }
}
