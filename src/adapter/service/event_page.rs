use getset::Getters;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::{event_seq::EventSeq, stack_event::StackEvent};

/// Events after a cursor, and the cursor to continue from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
#[serde(rename_all = "kebab-case")]
pub struct EventPage {
    /// The events, oldest first.
    events: Vec<StackEvent>,
    /// The newest position in the log, to pass as `since` next time.
    cursor: EventSeq,
    /// Whether events after `since` were dropped before they were read.
    gap: bool,
}
