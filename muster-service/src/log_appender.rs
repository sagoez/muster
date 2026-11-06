use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};

use super::{followed_stream::FollowedStream, state_dir::LOG_FILE_MODE};

/// Size past which the log is rotated before the next line: it moves to the
/// rotated name, replacing the older one, and a fresh log starts.
pub(super) const SERVICE_LOG_MAX_BYTES: u64 = 1024 * 1024;
/// Appended to the log's name for the generation rotated out.
const ROTATED_SUFFIX: &str = ".1";

/// The mirror thread's open log: one append handle and how many bytes the
/// log holds, so a line costs one write rather than a stat and an open.
pub(super) struct LogAppender {
    /// The live log, or `None` until an open succeeds.
    file: Option<File>,
    /// The log's size when opened plus every byte appended since.
    written: u64,
}

impl LogAppender {
    /// The appender for `log`, opened now when it can be.
    pub(super) fn open(log: &Path) -> Self {
        let mut appender = Self {
            file: None,
            written: 0,
        };
        appender.adopt(open_log(log).ok());
        appender
    }

    /// Takes `file` as the live log, its current size as the count.
    fn adopt(&mut self, file: Option<File>) {
        self.written = file
            .as_ref()
            .and_then(|file| file.metadata().ok())
            .map_or(0, |metadata| metadata.len());
        self.file = file;
    }

    /// Appends `line` to `log`, rotating first when it would carry the log
    /// past [`SERVICE_LOG_MAX_BYTES`]; a failed rotation reopens whatever
    /// sits at `log`. A failed write drops the handle, so the next line opens
    /// the log afresh.
    pub(super) fn append(&mut self, log: &Path, line: &[u8], stream: &mut dyn FollowedStream) {
        if self.file.is_none() {
            self.adopt(open_log(log).ok());
        }
        if self.written > 0
            && self.written.saturating_add(line.len() as u64) > SERVICE_LOG_MAX_BYTES
        {
            let fresh = rotate(log, stream).or_else(|| open_log(log).ok());
            self.adopt(fresh);
        }
        let Some(file) = self.file.as_mut() else {
            return;
        };
        if file.write_all(line).is_ok() {
            self.written = self.written.saturating_add(line.len() as u64);
        } else {
            self.file = None;
        }
    }
}

/// The name `log` is rotated to.
pub(super) fn rotated_log(log: &Path) -> PathBuf {
    let mut name = OsString::from(log.as_os_str());
    name.push(ROTATED_SUFFIX);
    PathBuf::from(name)
}

/// Moves `log` to its rotated name and starts a fresh one, pointing `stream`
/// at it when the stream wrote to the log rotated out, so its writes never
/// land in a file the next rotation unlinks. Returns the fresh log, or `None`
/// when the log could not be moved or reopened.
fn rotate(log: &Path, stream: &mut dyn FollowedStream) -> Option<File> {
    let followed = writes_to(stream, log);
    fs::rename(log, rotated_log(log)).ok()?;
    let fresh = open_log(log).ok()?;
    if followed {
        let _ = stream.repoint(&fresh);
    }
    Some(fresh)
}

/// Whether `stream` writes to the file at `log`.
fn writes_to(stream: &dyn FollowedStream, log: &Path) -> bool {
    let (Ok(target), Ok(log)) = (
        stream.target().and_then(|file| file.metadata()),
        fs::metadata(log),
    ) else {
        return false;
    };
    target.dev() == log.dev() && target.ino() == log.ino()
}

/// Opens `log` to append, creating it private to its owner.
///
/// # Errors
/// Returns the I/O error from opening it.
pub(super) fn open_log(log: &Path) -> io::Result<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .mode(LOG_FILE_MODE)
        .open(log)
}
