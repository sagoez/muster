use std::{fs, os::unix::fs::MetadataExt, path::Path};

use typed_builder::TypedBuilder;

/// Which file a path named when it was looked at: its device and inode,
/// which a file deleted and made again under the same name does not keep.
#[derive(Clone, Copy, Debug, PartialEq, Eq, TypedBuilder)]
pub(super) struct FileIdentity {
    /// The device it lives on.
    device: u64,
    /// Its inode on that device.
    inode: u64,
}

impl FileIdentity {
    /// The identity of the file at `path`; none when it is not there.
    pub(super) fn of(path: &Path) -> Option<Self> {
        fs::metadata(path).ok().map(|metadata| {
            Self::builder()
                .device(metadata.dev())
                .inode(metadata.ino())
                .build()
        })
    }
}
