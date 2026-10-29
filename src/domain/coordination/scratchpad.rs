use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::coordination::{Author, ScratchpadKey};

/// A durable, shared markdown note in a project: a named place where agents and
/// the human pass context and feedback to each other. The body is arbitrary
/// markdown; only the key is validated.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Scratchpad {
    /// The note's addressable name.
    key: ScratchpadKey,
    /// The note's markdown body.
    body: String,
    /// Who last wrote the note, stamped by the store.
    author: Author,
    /// Unix epoch seconds of the last write, stamped by the store.
    updated_at: u64,
}
