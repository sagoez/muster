use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

use super::event_view::EventView;
use crate::adapter::service::EventPage;

/// A page of stack events, as `stack_events` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct EventsView {
    /// The events, oldest first.
    #[getset(get = "pub")]
    events: Vec<EventView>,
    /// Pass as since next time.
    #[getset(get = "pub")]
    cursor: String,
    /// Whether older events were dropped before they could be read.
    #[getset(get_copy = "pub")]
    gap: bool,
}

impl From<&EventPage> for EventsView {
    fn from(page: &EventPage) -> Self {
        Self {
            events: page.events().iter().map(EventView::from).collect(),
            cursor: page.cursor().to_string(),
            gap: *page.gap(),
        }
    }
}
