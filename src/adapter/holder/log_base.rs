use std::{
    ffi::OsString,
    fs::{self, File},
    io::{self, Write},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

use atomic_write_file::AtomicWriteFile;
use getset::CopyGetters;
use typed_builder::TypedBuilder;

/// Suffix of the sidecar that records where the live log starts.
const BASE_SUFFIX: &str = ".base";
/// Separates the sidecar's offset from the file identity it belongs to.
const FIELD_SEPARATOR: char = ' ';

/// The path of the sidecar beside the log at `log`.
fn base_path(log: &Path) -> PathBuf {
    let mut base = OsString::from(log.as_os_str());
    base.push(BASE_SUFFIX);
    PathBuf::from(base)
}

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

/// Where a log's live file starts in the process's output stream, and which
/// file that was recorded for. The identity lets a reader, or a holder after
/// a crash, tell a record written for the current live file from one left
/// over from the file a rotation just renamed: a rotation renames first and
/// rewrites the sidecar second.
#[derive(Debug, Clone, Copy, PartialEq, Eq, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub(super)")]
pub(super) struct LogBase {
    /// The absolute offset of the live file's first byte.
    offset: u64,
    /// The inode of the live file it was recorded for.
    inode: u64,
}

/// A live file's start offset, and whether the sidecar pinned it exactly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, CopyGetters)]
#[getset(get_copy = "pub(super)")]
pub(super) struct Resolved {
    /// The absolute offset of the live file's first byte.
    offset: u64,
    /// Whether the sidecar belongs to the files found; otherwise the offset
    /// is the sidecar's own, the best available.
    exact: bool,
}

impl LogBase {
    /// The sidecar of the log at `log`, or none when it is absent or does not
    /// parse.
    pub(super) fn read(log: &Path) -> Option<Self> {
        let text = fs::read_to_string(base_path(log)).ok()?;
        let (offset, inode) = text.trim().split_once(FIELD_SEPARATOR)?;
        Some(Self {
            offset: offset.parse().ok()?,
            inode: inode.parse().ok()?,
        })
    }

    /// Replaces the sidecar of the log at `log` atomically.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when the sidecar cannot be written.
    pub(super) fn write(&self, log: &Path) -> io::Result<()> {
        let mut file = AtomicWriteFile::open(base_path(log))?;
        writeln!(file, "{}{FIELD_SEPARATOR}{}", self.offset, self.inode)?;
        file.commit()
    }

    /// Where the `live` file starts, given the sidecar found beside it and
    /// the `rotated` generation. With no sidecar the rotated file, if any,
    /// starts the stream at zero. A sidecar recorded for the rotated file
    /// predates the rotation that renamed it, so the live file starts where
    /// the rotated one ends.
    pub(super) fn resolve(
        sidecar: Option<Self>,
        live: Option<Generation>,
        rotated: Option<Generation>,
    ) -> Resolved {
        let rotated_len = rotated.map_or(0, |rotated| rotated.len);
        let (offset, exact) = match sidecar {
            None => (rotated_len, true),
            Some(base) if live.is_some_and(|live| live.inode == base.inode) => (base.offset, true),
            Some(base) if rotated.is_some_and(|rotated| rotated.inode == base.inode) => {
                (base.offset.saturating_add(rotated_len), true)
            },
            Some(base) => (base.offset, false),
        };
        Resolved { offset, exact }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    fn generation(inode: u64, len: u64) -> Option<Generation> {
        Some(Generation { inode, len })
    }

    #[test]
    fn a_sidecar_round_trips_and_garbage_reads_as_none() {
        let scratch = TempDir::new().unwrap();
        let log = scratch.path().join("api.log");
        assert_eq!(LogBase::read(&log), None);

        let base = LogBase {
            offset: 18234,
            inode: 77,
        };
        base.write(&log).unwrap();
        assert_eq!(LogBase::read(&log), Some(base));

        fs::write(base_path(&log), "not a base").unwrap();
        assert_eq!(LogBase::read(&log), None);
    }

    /// The live file's start follows the sidecar that belongs to it, or the
    /// rotated file's end when the sidecar predates the rotation.
    #[test]
    fn the_live_start_is_resolved_from_the_files_found() {
        let base = Some(LogBase {
            offset: 100,
            inode: 7,
        });
        let exact = |offset| Resolved {
            offset,
            exact: true,
        };

        assert_eq!(LogBase::resolve(None, generation(1, 5), None), exact(0));
        assert_eq!(
            LogBase::resolve(None, generation(1, 5), generation(2, 40)),
            exact(40)
        );
        assert_eq!(
            LogBase::resolve(base, generation(7, 5), generation(2, 40)),
            exact(100)
        );
        assert_eq!(
            LogBase::resolve(base, generation(8, 0), generation(7, 60)),
            exact(160)
        );
        assert_eq!(
            LogBase::resolve(base, generation(8, 0), generation(9, 60)),
            Resolved {
                offset: 100,
                exact: false
            }
        );
    }
}
