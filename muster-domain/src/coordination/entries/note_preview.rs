use getset::Getters;
use typed_builder::TypedBuilder;

use crate::{
    coordination::{Author, NoteBody, NoteBodyBytes, ScratchpadKey},
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
    body_start: NoteBody,
    /// How many bytes the whole body takes, never fewer than its start.
    body_bytes: NoteBodyBytes,
    /// Who last wrote the note.
    author: Author,
    /// When it was last written.
    updated_at: EpochSeconds,
}

impl NotePreview {
    /// How many bytes of the body the start leaves off; none when it is
    /// the whole body.
    pub fn unread_bytes(&self) -> usize {
        self.body_bytes
            .into_inner()
            .saturating_sub(self.body_start.as_ref().len())
    }

    /// Whether [`Self::body_start`] is the whole body.
    pub fn is_whole(&self) -> bool {
        self.unread_bytes() == 0
    }
}
