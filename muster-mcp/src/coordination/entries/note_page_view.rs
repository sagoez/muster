use std::fmt::Display;

use getset::{CopyGetters, Getters};
use muster_client::NoteEntry;
use serde::Serialize;
use typed_builder::TypedBuilder;

/// A page of the note index, as `note_read` without a key answers it.
#[derive(Debug, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct NotePageView {
    /// The notes on this page, without their bodies.
    #[getset(get = "pub")]
    notes: Vec<NoteEntry>,
    /// Whether more notes follow.
    #[getset(get_copy = "pub")]
    more: bool,
    /// Where the next page starts, when more follow.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[getset(get = "pub")]
    cursor: Option<String>,
}

impl NotePageView {
    /// `notes`, followed by more from `next` when there are any.
    pub fn of(notes: Vec<NoteEntry>, next: Option<impl Display>) -> Self {
        Self::builder()
            .notes(notes)
            .more(next.is_some())
            .cursor(next.map(|cursor| cursor.to_string()))
            .build()
    }
}
