use getset::Getters;
use muster_domain::value::{ProcessName, ProjectRoot};
use typed_builder::TypedBuilder;

/// Why one process failed, as the line shown under it: the process's own
/// text, made printable and cut to fit.
#[derive(Clone, Debug, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct FailureReason {
    /// The folder of the process's project.
    root: ProjectRoot,
    /// The process.
    name: ProcessName,
    /// The reason line.
    text: String,
}
