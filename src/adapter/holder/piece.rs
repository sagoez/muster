use std::fs::File;

use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

/// One generation's file and where it sits in the output stream.
#[derive(Getters, CopyGetters, TypedBuilder)]
pub(super) struct Piece {
    /// The open file; a rotation renaming it does not disturb the read.
    #[getset(get = "pub(super)")]
    file: File,
    /// The absolute offset of the first byte read from it.
    #[getset(get_copy = "pub(super)")]
    start: u64,
    /// Bytes read from it, at most its length when it was opened.
    #[getset(get_copy = "pub(super)")]
    len: u64,
    /// Bytes at the front of the file that fall before offset zero, which
    /// only a sidecar that disagrees with the files can leave.
    #[getset(get_copy = "pub(super)")]
    skip: u64,
}

impl Piece {
    /// Just past the last offset read from it.
    pub(super) fn end(&self) -> u64 {
        self.start + self.len
    }

    /// Whether absolute `offset` is read from it.
    pub(super) fn holds(&self, offset: u64) -> bool {
        offset >= self.start && offset < self.end()
    }
}
