use getset::Getters;
use muster_domain::{
    coordination::{Author, NoteHeader},
    value::EpochSeconds,
};
use serde::Serialize;
use typed_builder::TypedBuilder;

/// A note without its body: the index `note_read` and `muster note --json`
/// list, so a project full of long notes cannot flood the caller in one call.
#[derive(Debug, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct NoteEntry {
    /// The note's key.
    key: String,
    /// Who last wrote it.
    #[serde(with = "muster_protocol::wire::form")]
    author: Author,
    /// When it was last written, as RFC 3339 UTC.
    #[serde(with = "muster_protocol::wire::form")]
    updated_at: EpochSeconds,
}

impl From<&NoteHeader> for NoteEntry {
    fn from(note: &NoteHeader) -> Self {
        Self {
            key: note.key().as_ref().to_string(),
            author: note.author().clone(),
            updated_at: *note.updated_at(),
        }
    }
}
