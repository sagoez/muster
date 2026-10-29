use getset::Getters;
use serde::Serialize;
use typed_builder::TypedBuilder;

use crate::domain::coordination::Author;

/// A scratchpad without its body: the index `scratchpad_list` returns, so a
/// project full of long notes can never flood the caller in one call. The body
/// is fetched per key with `scratchpad_get`.
#[derive(Debug, Serialize, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ScratchpadEntry {
    /// The note's key.
    key: String,
    /// Who last wrote it.
    author: Author,
    /// When it was last written, in seconds since the Unix epoch.
    updated_at: u64,
}
