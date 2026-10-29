use serde::{Deserialize, Serialize};

use crate::{
    adapter::wire::{WireForm, form},
    domain::coordination::{Author, KV_VALUE_MAX_CHARS, KeyValue, KvKey, KvValue},
};

/// A key-value entry as a coordination snapshot and an agent read it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyValueEntry {
    /// The entry's name.
    key: KvKey,
    /// Its stored value.
    value: String,
    /// Who last wrote it.
    #[serde(with = "form")]
    author: Author,
    /// Unix epoch seconds of the last write.
    updated_at: u64,
}

impl WireForm for KeyValue {
    type Wire = KeyValueEntry;

    fn to_wire(&self) -> KeyValueEntry {
        KeyValueEntry {
            key: self.key().clone(),
            value: self.value().clone(),
            author: self.author().clone(),
            updated_at: *self.updated_at(),
        }
    }

    fn from_wire(wire: KeyValueEntry) -> Result<Self, String> {
        let found = wire.value.chars().count();
        let value = KvValue::try_new(wire.value).map_err(|_| {
            format!(
                "value '{}' is {found} characters; at most {KV_VALUE_MAX_CHARS} are allowed",
                wire.key.as_ref()
            )
        })?;
        Ok(Self::builder()
            .key(wire.key)
            .value(value.into_inner())
            .author(wire.author)
            .updated_at(wire.updated_at)
            .build())
    }
}
