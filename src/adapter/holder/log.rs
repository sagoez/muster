use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions, Permissions},
    io::{self, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use crate::{adapter::private_dir::create_private_dir_all, domain::value::ByteSize};

/// Suffix of the single retained older log generation.
const ROTATED_SUFFIX: &str = ".1";
/// Mode of a log: its owner's alone.
const PRIVATE_LOG_MODE: u32 = 0o600;

/// An append-only log that keeps one older generation: once the next write
/// would pass the cap, `name.log` becomes `name.log.1` and a fresh file starts.
pub struct RotatingLog {
    /// The live log file's path.
    path: PathBuf,
    /// The live log file, opened for appending.
    file: File,
    /// Bytes currently in the live file.
    written: u64,
    /// Size at which the live file rotates.
    cap: ByteSize,
}

impl RotatingLog {
    /// Opens `path` for appending, creating it and its directory as needed and
    /// counting any bytes already there toward the cap. Output can hold
    /// secrets, so the directories it creates and the log, narrowed when an
    /// older one is found open, are the owner's alone.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when the directory or file cannot be opened.
    pub fn open(path: PathBuf, cap: ByteSize) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            create_private_dir_all(parent)?;
        }
        let file = Self::append_to(&path)?;
        file.set_permissions(Permissions::from_mode(PRIVATE_LOG_MODE))?;
        let written = file.metadata()?.len();
        Ok(Self {
            path,
            file,
            written,
            cap,
        })
    }

    /// Appends `bytes`, rotating first when they would carry a non-empty live
    /// file past the cap. A single write larger than the cap is kept whole.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when rotating or writing fails.
    pub fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        let incoming = bytes.len() as u64;
        if self.written > 0 && self.written.saturating_add(incoming) > self.cap.bytes() {
            self.rotate()?;
        }
        self.file.write_all(bytes)?;
        self.written = self.written.saturating_add(incoming);
        Ok(())
    }

    /// Moves the live file to its rotated name, replacing the older generation,
    /// and starts an empty live file.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when the rename or reopen fails.
    fn rotate(&mut self) -> io::Result<()> {
        fs::rename(&self.path, rotated_path(&self.path))?;
        self.file = Self::append_to(&self.path)?;
        self.written = 0;
        Ok(())
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
}
