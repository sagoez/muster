use std::path::PathBuf;

use getset::Getters;
use muster_domain::value::{ProcessName, ProjectRoot};
use typed_builder::TypedBuilder;

/// A running process whose log the floor wants statted.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub(crate) struct LogWatch {
    /// Its project's folder.
    root: ProjectRoot,
    /// The process.
    name: ProcessName,
    /// Its log.
    path: PathBuf,
}
