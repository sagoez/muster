use getset::Getters;
use serde::Serialize;
use typed_builder::TypedBuilder;

use crate::domain::coordination::{Author, Scratchpad};

/// A note without its body: the index `note_read` lists, so a project full
/// of long notes cannot flood the caller in one call.
#[derive(Debug, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct NoteEntry {
    /// The note's key.
    key: String,
    /// Who last wrote it.
    #[serde(with = "crate::adapter::wire::form")]
    author: Author,
    /// When it was last written, in seconds since the Unix epoch.
    updated_at: u64,
}

impl From<&Scratchpad> for NoteEntry {
    fn from(note: &Scratchpad) -> Self {
        Self {
            key: note.key().as_ref().to_string(),
            author: note.author().clone(),
            updated_at: *note.updated_at(),
        }
    }
}
