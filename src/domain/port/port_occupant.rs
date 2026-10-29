use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use crate::domain::value::Pid;

/// A process listening on a TCP port, found in the OS socket table.
#[derive(Debug, Clone, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub struct PortOccupant {
    /// Its process id.
    #[getset(get_copy = "pub")]
    pid: Pid,
    /// Its command line, or its executable when the command line is unknown.
    #[getset(get = "pub")]
    command: String,
}
