use getset::Getters;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use crate::domain::value::{ProjectName, ProjectRoot};

/// A registered project: a folder, which is its identity, and its display name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Project {
    /// The canonical folder the project is.
    root: ProjectRoot,
    /// The name the CLI shows and accepts for it.
    name: ProjectName,
}
