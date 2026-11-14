use std::path::PathBuf;

use getset::Getters;
use muster_domain::value::{ProcessName, ProjectName};
use typed_builder::TypedBuilder;

/// A running process whose log the floor wants statted.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct LogWatch {
    /// Its project.
    project: ProjectName,
    /// The process.
    name: ProcessName,
    /// Its log.
    path: PathBuf,
}
