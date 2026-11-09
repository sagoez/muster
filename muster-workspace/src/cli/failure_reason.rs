use getset::{CopyGetters, Getters};
use muster_domain::value::{ProcessName, ProjectRoot};
use typed_builder::TypedBuilder;

/// Why one process failed, as the line shown under it: the process's own
/// text, made printable and cut to fit.
#[derive(Clone, Debug, Getters, CopyGetters, TypedBuilder)]
pub struct FailureReason {
    /// The folder of the process's project.
    #[getset(get = "pub")]
    root: ProjectRoot,
    /// The process.
    #[getset(get = "pub")]
    name: ProcessName,
    /// The reason line, cut to fit.
    #[getset(get = "pub")]
    text: String,
    /// The reason whole, before the cut, to tell it apart from what a row
    /// already says.
    #[getset(get = "pub")]
    whole: String,
    /// Whether its run never launched, so it logged nothing to read.
    #[getset(get_copy = "pub")]
    never_launched: bool,
}
