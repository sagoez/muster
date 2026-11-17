use getset::Getters;
use typed_builder::TypedBuilder;

use crate::{
    coordination::{Author, ScratchpadKey},
    value::EpochSeconds,
};

/// A note as a short read shows it: its key, who last wrote it and when,
/// the start of its body, and how long the whole body is.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct NotePreview {
    /// The note's addressable name.
    key: ScratchpadKey,
    /// The start of the note's body, the whole of it when short enough.
    body_start: String,
    /// How many bytes the whole body takes.
    body_bytes: usize,
    /// Who last wrote the note.
    author: Author,
    /// When it was last written.
    updated_at: EpochSeconds,
}

impl NotePreview {
    /// Whether [`Self::body_start`] is the whole body.
    pub fn is_whole(&self) -> bool {
        self.body_start.len() >= self.body_bytes
    }
}
