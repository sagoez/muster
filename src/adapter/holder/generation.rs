use std::{fs::File, io, os::unix::fs::MetadataExt};

use getset::CopyGetters;
use typed_builder::TypedBuilder;

/// One generation of a log as found on disk: its file identity and length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub(super)")]
pub(super) struct Generation {
    /// The file's inode, which a rename keeps.
    inode: u64,
    /// Bytes in the file.
    len: u64,
}

impl Generation {
    /// The identity and length `file` has now.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when the file cannot be inspected.
    pub(super) fn of(file: &File) -> io::Result<Self> {
        let metadata = file.metadata()?;
        Ok(Self {
            inode: metadata.ino(),
            len: metadata.len(),
        })
    }
}
