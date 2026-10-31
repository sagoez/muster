use std::{
    fs::{self, Metadata},
    io,
    os::unix::fs::MetadataExt,
    path::Path,
};

use getset::Getters;
use typed_builder::TypedBuilder;

/// What tells a config file apart from a later version of it without
/// reading it again: which file it is, its size, and when it was last
/// written or had its details changed, the last of which no writer can set
/// back.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Getters, TypedBuilder)]
pub(super) struct FileFingerprint {
    /// The device holding the file.
    device: u64,
    /// The file's inode, which a rename onto its path replaces.
    inode: u64,
    /// Its size in bytes.
    size: u64,
    /// When its content was last written, in seconds and nanoseconds.
    modified: (i64, i64),
    /// When its content or details last changed, in seconds and
    /// nanoseconds.
    changed: (i64, i64),
}

impl FileFingerprint {
    /// The fingerprint of the file `metadata` describes.
    pub(super) fn of(metadata: &Metadata) -> Self {
        Self::builder()
            .device(metadata.dev())
            .inode(metadata.ino())
            .size(metadata.size())
            .modified((metadata.mtime(), metadata.mtime_nsec()))
            .changed((metadata.ctime(), metadata.ctime_nsec()))
            .build()
    }

    /// The fingerprint of what is at `path` now, without following a link
    /// there, or none when nothing is.
    ///
    /// # Errors
    /// Returns the failure to look at `path`.
    pub(super) fn at(path: &Path) -> io::Result<Option<Self>> {
        match fs::symlink_metadata(path) {
            Ok(metadata) => Ok(Some(Self::of(&metadata))),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    /// A file rewritten in place or replaced by a rename is told apart, and
    /// a missing one has no fingerprint.
    #[test]
    fn a_rewrite_or_a_rename_changes_the_fingerprint() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("config.json");
        assert_eq!(FileFingerprint::at(&path).unwrap(), None);
        fs::write(&path, "{}").unwrap();
        let first = FileFingerprint::at(&path).unwrap().unwrap();
        assert_eq!(FileFingerprint::at(&path).unwrap(), Some(first));

        fs::write(&path, "{\"a\": 1}").unwrap();
        let rewritten = FileFingerprint::at(&path).unwrap().unwrap();
        assert_ne!(rewritten, first);

        let beside = dir.path().join("new.json");
        fs::write(&beside, "{\"a\": 1}").unwrap();
        fs::rename(&beside, &path).unwrap();
        assert_ne!(FileFingerprint::at(&path).unwrap(), Some(rewritten));
    }
}
