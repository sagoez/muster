use getset::Getters;
use muster_domain::coordination::{Scratchpad, ScratchpadKey};
use serde::Serialize;
use typed_builder::TypedBuilder;

use crate::AgentMark;

/// One shared note as the notes panel shows it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct NoteView {
    /// The note's name.
    key: ScratchpadKey,
    /// Its markdown body, as plain text.
    body: String,
    /// The mark of who last wrote it.
    author: AgentMark,
    /// Who last wrote it, as the store labels them.
    label: String,
    /// When it was last written, in seconds since the Unix epoch.
    at: u64,
}

impl From<&Scratchpad> for NoteView {
    fn from(note: &Scratchpad) -> Self {
        Self::builder()
            .key(note.key().clone())
            .body(note.body().clone())
            .author(AgentMark::of(note.author()))
            .label(note.author().label().to_string())
            .at(note.updated_at().into_inner())
            .build()
    }
}
