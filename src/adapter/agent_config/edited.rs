use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use super::edit_outcome::EditOutcome;

/// One file's text after an entry was added or removed.
#[derive(Clone, Debug, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub(super) struct Edited {
    /// What the edit did.
    #[getset(get_copy = "pub(super)")]
    outcome: EditOutcome,
    /// The file's new text, when the edit changes it.
    #[getset(get = "pub(super)")]
    text: Option<String>,
}
