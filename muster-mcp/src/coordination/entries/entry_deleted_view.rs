use getset::{CopyGetters, Getters};
use serde::Serialize;
use typed_builder::TypedBuilder;

/// What `note_delete` and `kv_delete` answer: the key named, and whether an
/// entry was stored under it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Getters, CopyGetters, TypedBuilder)]
pub struct EntryDeletedView {
    /// The entry's key.
    #[getset(get = "pub")]
    key: String,
    /// Whether an entry was deleted; false when there was none.
    #[getset(get_copy = "pub")]
    deleted: bool,
}
