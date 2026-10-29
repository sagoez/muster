use getset::Getters;
use serde::Serialize;
use typed_builder::TypedBuilder;

use crate::adapter::{
    sanitize::printable,
    service::{EventKind, StackEvent},
};

/// One stack event as an agent reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct EventView {
    /// Its cursor; pass it as since to read later events.
    seq: String,
    /// When it happened, as RFC 3339 UTC to the second.
    at: String,
    /// The process it concerns.
    #[serde(skip_serializing_if = "Option::is_none")]
    process: Option<String>,
    /// What happened.
    kind: EventKind,
    /// A short description, such as `exit 1`.
    detail: String,
    /// The process's output cursor when it happened.
    #[serde(skip_serializing_if = "Option::is_none")]
    cursor: Option<String>,
}

impl From<&StackEvent> for EventView {
    fn from(event: &StackEvent) -> Self {
        Self {
            seq: event.seq().to_string(),
            at: humantime::format_rfc3339_seconds(*event.at()).to_string(),
            process: event.process().as_ref().map(ToString::to_string),
            kind: event.kind(),
            detail: printable(event.detail()),
            cursor: event.cursor().map(|cursor| cursor.to_string()),
        }
    }
}
