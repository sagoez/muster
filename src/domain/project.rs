use std::path::PathBuf;

use getset::Getters;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use crate::domain::value::ProjectName;

/// A registered project: a display name and the path to its workspace config.
#[derive(Clone, Debug, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Project {
    name: ProjectName,
    config: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_name_and_config() {
        let project: Project =
            serde_yaml_ng::from_str("name: muster\nconfig: ~/Projects/muster/muster.yml").unwrap();
        assert_eq!(project.name().as_ref(), "muster");
        assert_eq!(
            project.config(),
            &PathBuf::from("~/Projects/muster/muster.yml")
        );
    }
}
