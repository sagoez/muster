use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::{
    coordination::{Author, ScratchpadKey},
    value::EpochSeconds,
};

/// A note without its body: what an index of notes lists, so paging through
/// long notes never reads them.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct NoteHeader {
    /// The note's addressable name.
    key: ScratchpadKey,
    /// Who last wrote the note.
    author: Author,
    /// When it was last written.
    updated_at: EpochSeconds,
}
