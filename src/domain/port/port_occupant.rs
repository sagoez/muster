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
    /// Its parent, that parent's parent and so on, as far as they were found,
    /// so a listener a supervised process spawned is told to be its.
    #[getset(get = "pub")]
    #[builder(default)]
    ancestors: Vec<Pid>,
}

impl PortOccupant {
    /// Its pid, then its ancestors': every process it may have been
    /// started under.
    pub fn lineage(&self) -> Vec<Pid> {
        let mut lineage = vec![self.pid];
        lineage.extend(self.ancestors.iter().copied());
        lineage
    }
}
