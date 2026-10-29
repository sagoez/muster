use getset::{CopyGetters, Getters};
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use crate::domain::value::Pid;

/// A process listening on a TCP port, found in the OS socket table.
#[derive(
    Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Getters, CopyGetters, TypedBuilder,
)]
#[serde(rename_all = "kebab-case")]
pub struct PortOccupant {
    /// Its process id.
    #[getset(get_copy = "pub")]
    pid: Pid,
    /// Its command line, or its executable when the command line is unknown.
    #[getset(get = "pub")]
    command: String,
}

/// Finds who listens on a port, whoever started it. A driven port so the
/// domain never names the OS interface that lists sockets; `&mut self` lets
/// an implementation keep a process table between lookups.
pub trait PortOccupants {
    /// A process listening on TCP `port`, if any.
    fn occupant(&mut self, port: u16) -> Option<PortOccupant>;
}
