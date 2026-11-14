use std::path::PathBuf;

use getset::Getters;
use muster_domain::{project::Project, value::ProcessName};
use typed_builder::TypedBuilder;

/// The log of one process on the floor, found through the floor's own
/// project and process, never through a request's text.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct LocatedLog {
    /// The process's project.
    project: Project,
    /// The process, as its stack names it.
    name: ProcessName,
    /// Its log, where the holders' layout puts it.
    path: PathBuf,
}
