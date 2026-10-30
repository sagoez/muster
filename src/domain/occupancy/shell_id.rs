use getset::CopyGetters;
use typed_builder::TypedBuilder;

use crate::domain::value::{Pid, StartToken};

/// One interactive shell: its pid and that process's start token, so a later
/// process reusing the pid is a different shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct ShellId {
    /// The shell's process id.
    pid: Pid,
    /// When that process started, as the OS marks it.
    start: StartToken,
}
