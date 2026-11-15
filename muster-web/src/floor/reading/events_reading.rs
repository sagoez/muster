use getset::{CopyGetters, Getters};
use muster_protocol::service::StackEvent;
use typed_builder::TypedBuilder;

/// What one page of the service's event log told the floor, read after a
/// cursor: a page read from no cursor holds only history, so it tells
/// nothing.
#[derive(Clone, Debug, PartialEq, Getters, CopyGetters, TypedBuilder)]
pub(crate) struct EventsReading {
    /// The events after the cursor, oldest first.
    #[getset(get = "pub")]
    events: Vec<StackEvent>,
    /// Whether events after the cursor were dropped before they were read.
    #[getset(get_copy = "pub")]
    gap: bool,
}
