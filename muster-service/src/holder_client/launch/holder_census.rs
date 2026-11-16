use std::collections::HashSet;

use getset::Getters;
use muster_protocol::holder::{HolderId, HolderRecord};
use typed_builder::TypedBuilder;

/// Every holder record as one look at service start found it, with whose
/// holders still run and which records are worth keeping, worked out once,
/// so each pass over the records shares one answer and none asks the system
/// again.
#[derive(Debug, Clone, Getters, TypedBuilder)]
pub struct HolderCensus {
    /// Every record that could be read.
    #[getset(get = "pub")]
    records: Vec<HolderRecord>,
    /// The holders still running as the process their record names.
    #[getset(get = "pub")]
    alive: HashSet<HolderId>,
    /// The records worth adopting: of a holder still running, of a dead
    /// holder whose child runs on, or of one that left its child's exit.
    #[getset(get = "pub")]
    kept: HashSet<HolderId>,
}
