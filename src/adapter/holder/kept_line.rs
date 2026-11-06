use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use crate::domain::value::OutputCursor;

/// One entry a read keeps: a line of output, or a separator it put between
/// two error groups, told apart by how it was added and never by its text.
#[derive(Clone, Debug, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub(super) struct KeptLine {
    /// The text.
    #[getset(get = "pub(super)")]
    text: String,
    /// Where the line, or the group a separator opens, begins in the output.
    #[getset(get_copy = "pub(super)")]
    at: OutputCursor,
    /// Whether this is a group separator rather than a line of output.
    #[getset(get_copy = "pub(super)")]
    separator: bool,
}

impl KeptLine {
    /// The text, taken.
    pub(super) fn into_text(self) -> String {
        self.text
    }
}
