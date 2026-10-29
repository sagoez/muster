use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions, Permissions},
    io::{self, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use super::{generation::Generation, log_base::LogBase};
use crate::{
    adapter::private_dir::create_private_dir_all,
    domain::value::{ByteSize, OutputCursor},
};

/// Suffix of the single retained older log generation.
const ROTATED_SUFFIX: &str = ".1";
/// Mode of a log: its owner's alone.
const PRIVATE_LOG_MODE: u32 = 0o600;

/// An append-only log that keeps one older generation: once the next write
/// would pass the cap, `name.log` becomes `name.log.1` and a fresh file starts.
/// It also tracks where each byte sits in the process's output stream across
/// rotations and runs, recording the live file's start in a `.base` sidecar.
pub struct RotatingLog {
    /// The live log file's path.
    path: PathBuf,
    /// The live log file, opened for appending.
    file: File,
    /// Bytes currently in the live file.
    written: u64,
    /// The absolute offset of the live file's first byte.
    base: u64,
    /// Size at which the live file rotates.
    cap: ByteSize,
}

impl RotatingLog {
    /// Opens `path` for appending, creating it and its directory as needed and
    /// counting any bytes already there toward the cap, and continues the
    /// output offsets of every earlier run. Output can hold secrets, so the
    /// directories it creates and the log, narrowed when an older one is found
    /// open, are the owner's alone.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when the directory, file or sidecar cannot be
    /// opened or written.
    pub fn open(path: PathBuf, cap: ByteSize) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            create_private_dir_all(parent)?;
        }
        let file = Self::append_to(&path)?;
        file.set_permissions(Permissions::from_mode(PRIVATE_LOG_MODE))?;
        let live = Generation::of(&file)?;
        let rotated = match File::open(rotated_path(&path)) {
            Ok(rotated) => Some(Generation::of(&rotated)?),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        let sidecar = LogBase::read(&path);
        let base = LogBase::resolve(sidecar, Some(live), rotated).offset();
        let log = Self {
            path,
            file,
            written: live.len(),
            base,
            cap,
        };
        if sidecar.is_none_or(|sidecar| sidecar.offset() != base || sidecar.inode() != live.inode())
        {
            log.record_base()?;
        }
        Ok(log)
    }

    /// The position just past the last byte written: where the next append
    /// lands in the output stream.
    pub fn end(&self) -> OutputCursor {
        OutputCursor::new(self.base.saturating_add(self.written))
    }

    /// Appends `bytes`, rotating first when they would carry a non-empty live
    /// file past the cap. A single write larger than the cap is kept whole.
    /// After a failed write the length is taken from the file again, so
    /// [`Self::end`] keeps naming real log positions.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when rotating or writing fails.
    pub fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        let incoming = bytes.len() as u64;
        if self.written > 0 && self.written.saturating_add(incoming) > self.cap.bytes() {
            self.rotate()?;
        }
        if let Err(error) = self.file.write_all(bytes) {
            if let Ok(metadata) = self.file.metadata() {
                self.written = metadata.len();
            }
            return Err(error);
        }
        self.written = self.written.saturating_add(incoming);
        Ok(())
    }

    /// Moves the live file to its rotated name, replacing the older generation,
    /// starts an empty live file where the rotated one ends, and records that
    /// start.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when the rename, reopen or sidecar write fails.
    fn rotate(&mut self) -> io::Result<()> {
        fs::rename(&self.path, rotated_path(&self.path))?;
        self.file = Self::append_to(&self.path)?;
        self.base = self.base.saturating_add(self.written);
        self.written = 0;
        self.record_base()
    }

    /// Writes the sidecar naming the live file and its start.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when the file cannot be inspected or the
    /// sidecar written.
    fn record_base(&self) -> io::Result<()> {
        LogBase::builder()
            .offset(self.base)
            .inode(Generation::of(&self.file)?.inode())
            .build()
            .write(&self.path)
    }

    /// Opens `path` for appending, creating it when absent.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when the file cannot be opened.
    fn append_to(path: &Path) -> io::Result<File> {
        OpenOptions::new()
            .create(true)
            .append(true)
            .mode(PRIVATE_LOG_MODE)
            .open(path)
    }
}

/// The path of the older generation kept beside `path`.
pub fn rotated_path(path: &Path) -> PathBuf {
    let mut rotated = OsString::from(path.as_os_str());
    rotated.push(ROTATED_SUFFIX);
    PathBuf::from(rotated)
}

#[cfg(test)]
mod tests {
    use bytesize::ByteSize as RawByteSize;
    use tempfile::TempDir;

    use super::*;

    /// Mode bits a permission check compares.
    const PERMISSION_BITS: u32 = 0o777;
    /// The mode of a file only its owner can read.
    const OWNER_FILE: u32 = 0o600;
    /// The mode of a directory only its owner can enter.
    const OWNER_DIR: u32 = 0o700;
    /// A file mode other users can read.
    const SHARED_FILE: u32 = 0o644;

    /// The permission bits of `path`.
    fn mode_of(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & PERMISSION_BITS
    }

    /// A process's output can hold secrets, so its log, the rotated one and
    /// the directories created for them are its owner's alone, and an older
    /// log other users could read is narrowed when reopened.
    #[test]
    fn logs_and_their_directories_are_private() {
        let scratch = TempDir::new().unwrap();
        let path = scratch.path().join("logs").join("project").join("proc.log");
        let cap = ByteSize::try_new(RawByteSize::b(4)).unwrap();
        let mut log = RotatingLog::open(path.clone(), cap).unwrap();
        log.append(b"1234").unwrap();
        log.append(b"5678").unwrap();

        assert_eq!(mode_of(&path), OWNER_FILE);
        assert_eq!(mode_of(&rotated_path(&path)), OWNER_FILE);
        for dir in [
            path.parent().unwrap(),
            scratch.path().join("logs").as_path(),
        ] {
            assert_eq!(mode_of(dir), OWNER_DIR, "{}", dir.display());
        }

        drop(log);
        fs::set_permissions(&path, Permissions::from_mode(SHARED_FILE)).unwrap();
        RotatingLog::open(path.clone(), cap).unwrap();
        assert_eq!(mode_of(&path), OWNER_FILE);
    }

    #[test]
    fn log_rotates_at_cap() {
        let scratch = TempDir::new().unwrap();
        let path = scratch.path().join("logs").join("proc.log");
        let cap = ByteSize::try_new(RawByteSize::b(10)).unwrap();
        let mut log = RotatingLog::open(path.clone(), cap).unwrap();
        log.append(b"12345").unwrap();
        log.append(b"678").unwrap();
        log.append(b"abcdef").unwrap();

        assert_eq!(fs::read(rotated_path(&path)).unwrap(), b"12345678");
        assert_eq!(fs::read(&path).unwrap(), b"abcdef");

        drop(log);
        let mut reopened = RotatingLog::open(path.clone(), cap).unwrap();
        reopened.append(b"ghijk").unwrap();
        assert_eq!(
            fs::read(rotated_path(&path)).unwrap(),
            b"abcdef",
            "a reopened log counts its existing bytes and keeps one generation"
        );
        assert_eq!(fs::read(&path).unwrap(), b"ghijk");
    }

    /// Offsets run on across rotations and across runs that reopen the log,
    /// and the sidecar names where the live file starts.
    #[test]
    fn offsets_continue_across_rotations_and_runs() {
        let scratch = TempDir::new().unwrap();
        let path = scratch.path().join("logs").join("proc.log");
        let cap = ByteSize::try_new(RawByteSize::b(10)).unwrap();
        let mut log = RotatingLog::open(path.clone(), cap).unwrap();
        assert_eq!(log.end(), OutputCursor::new(0));
        log.append(b"12345678").unwrap();
        log.append(b"abcdef").unwrap();
        assert_eq!(log.end(), OutputCursor::new(14));
        assert_eq!(LogBase::read(&path).unwrap().offset(), 8);

        drop(log);
        let mut reopened = RotatingLog::open(path.clone(), cap).unwrap();
        assert_eq!(reopened.end(), OutputCursor::new(14), "a new run continues");
        reopened.append(b"ghijk").unwrap();
        assert_eq!(reopened.end(), OutputCursor::new(19));
        assert_eq!(LogBase::read(&path).unwrap().offset(), 14);
        assert_eq!(fs::read(rotated_path(&path)).unwrap(), b"abcdef");
    }

    /// A holder that died between a rotation's rename and its sidecar write
    /// leaves a sidecar naming the rotated file; the next run starts the live
    /// file where the rotated one ends rather than reusing its offsets.
    #[test]
    fn a_rotation_cut_short_before_its_sidecar_is_recovered() {
        let scratch = TempDir::new().unwrap();
        let path = scratch.path().join("proc.log");
        let cap = ByteSize::try_new(RawByteSize::b(10)).unwrap();
        let mut log = RotatingLog::open(path.clone(), cap).unwrap();
        log.append(b"12345678").unwrap();
        drop(log);
        fs::rename(&path, rotated_path(&path)).unwrap();

        let mut reopened = RotatingLog::open(path.clone(), cap).unwrap();
        assert_eq!(reopened.end(), OutputCursor::new(8));
        reopened.append(b"ab").unwrap();
        assert_eq!(reopened.end(), OutputCursor::new(10));
        assert_eq!(LogBase::read(&path).unwrap().offset(), 8);
    }
}
