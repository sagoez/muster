use getset::Getters;
use nutype::nutype;
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use crate::domain::coordination::Author;

/// Name addressing a shared key-value entry within a project. Trimmed and
/// non-empty, so a blank key can never address a value.
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
pub struct KvKey(String);

/// A shared, durable key-value entry in a project: the low-ceremony scratch
/// state agents pass between each other (a chosen port, a build id, a flag). The
/// value is an arbitrary string; only the key is validated. The store stamps
/// `updated_at`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct KeyValue {
    /// The entry's addressable name.
    key: KvKey,
    /// The entry's stored value.
    value: String,
    /// Who last wrote the entry, stamped by the store.
    author: Author,
    /// Unix epoch seconds of the last write, stamped by the store.
    updated_at: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A key is trimmed; a blank key is rejected.
    #[test]
    fn a_key_is_trimmed_and_blank_is_rejected() {
        assert_eq!(KvKey::try_new("  port  ").unwrap().as_ref(), "port");
        assert!(KvKey::try_new("   ").is_err());
    }
}
