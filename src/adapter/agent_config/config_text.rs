use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

/// A config file's text as muster read it, with the size the file had on
/// disk, so a later look can tell a rewrite apart cheaply before comparing
/// the text itself.
#[derive(Clone, Debug, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub(super) struct ConfigText {
    /// The text.
    #[getset(get = "pub(super)")]
    text: String,
    /// The file's size in bytes when it was read.
    #[getset(get_copy = "pub(super)")]
    size: u64,
}
