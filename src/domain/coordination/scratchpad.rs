use getset::Getters;
use nutype::nutype;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use crate::domain::coordination::Author;

/// Name identifying a scratchpad within a project. Trimmed and non-empty, so a
/// blank or whitespace-only key can never address a note.
#[nutype(
    sanitize(trim),
    validate(not_empty),
    derive(
        Debug,
        Clone,
        PartialEq,
        Eq,
        Hash,
        AsRef,
        Display,
        Serialize,
        Deserialize
    )
)]
pub struct ScratchpadKey(String);

/// A durable, shared markdown note in a project: a named place where agents and
/// the human pass context and feedback to each other. The body is arbitrary
/// markdown; only the key is validated.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Getters, TypedBuilder)]
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A key is trimmed of surrounding whitespace.
    #[test]
    fn a_key_is_trimmed() {
        assert_eq!(ScratchpadKey::try_new("  plan  ").unwrap().as_ref(), "plan");
    }

    /// A blank or whitespace-only key is rejected.
    #[test]
    fn a_blank_key_is_rejected() {
        assert!(ScratchpadKey::try_new("   ").is_err());
        assert!(ScratchpadKey::try_new("").is_err());
    }
}
