use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

/// A config file's text as muster read it, with what the file was on disk
/// then, so a later look can tell a rewrite, a file put in its place or a
/// change of mode apart cheaply before comparing the text itself.
#[derive(Clone, Debug, PartialEq, Eq, Getters, CopyGetters, TypedBuilder)]
pub(super) struct ConfigText {
    /// The text.
    #[getset(get = "pub(super)")]
    text: String,
    /// The file's size in bytes when it was read.
    #[getset(get_copy = "pub(super)")]
    size: u64,
    /// The device holding the file.
    #[getset(get_copy = "pub(super)")]
    device: u64,
    /// The file's inode, which another file put in its place does not
    /// share.
    #[getset(get_copy = "pub(super)")]
    inode: u64,
    /// The file's mode, which a rewrite keeps.
    #[getset(get_copy = "pub(super)")]
    mode: u32,
}
