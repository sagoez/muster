use getset::Getters;
use typed_builder::TypedBuilder;

use crate::domain::coordination::{Author, KvKey};

/// A shared, durable key-value entry in a project: the low-ceremony scratch
/// state agents pass between each other (a chosen port, a build id, a flag). The
/// value is an arbitrary string; only the key is validated. The store stamps
/// `updated_at`.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
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
