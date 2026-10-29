use getset::Getters;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::{WireForm, optional_form};
use crate::domain::{
    project::Project,
    value::{IdleStop, ProjectName, ProjectRoot},
};

/// A registered project, as the service reports it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct ProjectWire {
    /// The folder the project is.
    root: ProjectRoot,
    /// Its display name.
    name: ProjectName,
    /// Its own idle stop, or none to follow the setting.
    #[serde(with = "optional_form")]
    idle_stop: Option<IdleStop>,
}

impl WireForm for Project {
    type Wire = ProjectWire;

    fn to_wire(&self) -> ProjectWire {
        ProjectWire {
            root: self.root().clone(),
            name: self.name().clone(),
            idle_stop: *self.idle_stop(),
        }
    }

    fn from_wire(wire: ProjectWire) -> Result<Self, String> {
        Ok(Self::builder()
            .root(wire.root)
            .name(wire.name)
            .idle_stop(wire.idle_stop)
            .build())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// A project travels with its own idle stop, or `null` when it follows
    /// the setting.
    #[test]
    fn a_project_travels_with_its_idle_stop() {
        let project = |idle_stop| {
            Project::builder()
                .root(ProjectRoot::try_new(PathBuf::from("/home/me/shop")).unwrap())
                .name(ProjectName::try_new("shop").unwrap())
                .idle_stop(idle_stop)
                .build()
        };
        for (idle_stop, json) in [
            (
                Some(IdleStop::Never),
                r#"{"root":"/home/me/shop","name":"shop","idle-stop":"never"}"#,
            ),
            (
                None,
                r#"{"root":"/home/me/shop","name":"shop","idle-stop":null}"#,
            ),
        ] {
            let wire = project(idle_stop).to_wire();

            assert_eq!(serde_json::to_string(&wire).unwrap(), json);
            let read: ProjectWire = serde_json::from_str(json).unwrap();
            assert_eq!(Project::from_wire(read).unwrap(), project(idle_stop));
        }
    }
}
