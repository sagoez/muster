use getset::Getters;
use typed_builder::TypedBuilder;

use super::pause::Pause;

/// How each poller waits, as the hub hands them out: the projects keep
/// their pace for as long as the hub lives, while the status, the events,
/// the notes and the logs, read only for the pages, park while none
/// streams.
#[derive(Clone, Debug, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Pauses {
    /// The projects' poller's, until the hub is gone.
    life: Pause,
    /// The status poller's, also woken when an event asks for a fresh status.
    status: Pause,
    /// The events poller's.
    events: Pause,
    /// The notes poller's.
    notes: Pause,
    /// The logs poller's.
    logs: Pause,
}
