use getset::Getters;
use serde::Serialize;
use typed_builder::TypedBuilder;

/// What `note_write` and `kv_write` answer: the key the entry was saved under.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct SavedView {
    /// The entry's key.
    saved: String,
}
