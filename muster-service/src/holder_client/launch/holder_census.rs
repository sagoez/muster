use std::collections::HashSet;

use getset::Getters;
use muster_protocol::holder::{HolderId, HolderRecord};
use typed_builder::TypedBuilder;

/// The holder records worth adopting as one look found them, with whose
/// holders still run and which left their child's exit, worked out once, so
/// each pass over the records shares one answer and none asks the system
/// again.
#[derive(Debug, Clone, Getters, TypedBuilder)]
pub struct HolderCensus {
    /// The records worth adopting, in the order to adopt them: of holders
    /// still running, then of dead holders whose child runs on, then of
    /// holders that left their child's exit, so a live copy of a process is
    /// always adopted before an exit left of it.
    #[getset(get = "pub")]
    records: Vec<HolderRecord>,
    /// The holders still running as the process their record names.
    #[getset(get = "pub")]
    alive: HashSet<HolderId>,
    /// The holders that left their child's exit beside their record.
    #[getset(get = "pub")]
    stranded: HashSet<HolderId>,
}
