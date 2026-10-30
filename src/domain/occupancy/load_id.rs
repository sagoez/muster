use getset::CopyGetters;
use typed_builder::TypedBuilder;

use super::LoadStart;
use crate::domain::value::Pid;

/// Which loading of muster's hook in a shell made a report. A shell that
/// `exec`s itself keeps its pid and start token but loads the hook again,
/// numbering its prompts from one under a later load id. Ids order by when
/// the load began, then by the pid of a process the load spawned, which
/// breaks a tie of a coarse clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct LoadId {
    /// When the load began.
    started: LoadStart,
    /// The pid of a process the load spawned.
    pid: Pid,
}
