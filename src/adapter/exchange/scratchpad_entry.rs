use serde::{Deserialize, Serialize};

use crate::{
    adapter::wire::{WireForm, form},
    domain::coordination::{Author, NOTE_BODY_MAX_CHARS, NoteBody, Scratchpad, ScratchpadKey},
};

/// A scratchpad as a coordination snapshot and an agent read it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScratchpadEntry {
    /// The note's name.
    key: ScratchpadKey,
    /// Its markdown body.
    body: String,
    /// Who last wrote it.
    #[serde(with = "form")]
    author: Author,
    /// Unix epoch seconds of the last write.
    updated_at: u64,
}

impl WireForm for Scratchpad {
    type Wire = ScratchpadEntry;

    fn to_wire(&self) -> ScratchpadEntry {
        ScratchpadEntry {
            key: self.key().clone(),
            body: self.body().clone(),
            author: self.author().clone(),
            updated_at: *self.updated_at(),
        }
    }

    fn from_wire(wire: ScratchpadEntry) -> Result<Self, String> {
        let found = wire.body.chars().count();
        let body = NoteBody::try_new(wire.body).map_err(|_| {
            format!(
                "note '{}' has a body of {found} characters; at most {NOTE_BODY_MAX_CHARS} are allowed",
                wire.key.as_ref()
            )
        })?;
        Ok(Self::builder()
            .key(wire.key)
            .body(body.into_inner())
            .author(wire.author)
            .updated_at(wire.updated_at)
            .build())
    }
}
