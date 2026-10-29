use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use crossbeam_channel::{Receiver, Sender, bounded, unbounded};

use super::{followed_stream::FollowedStream, mirror_write::MirrorWrite, state_dir::LOG_FILE_MODE};

/// Size past which the log is rotated before the next line: it moves to the
/// rotated name, replacing the older one, and a fresh log starts.
const SERVICE_LOG_MAX_BYTES: u64 = 1024 * 1024;
/// Appended to the log's name for the generation rotated out.
const ROTATED_SUFFIX: &str = ".1";
/// Longest [`NoticeMirror::flush`] waits for the lines before it.
const FLUSH_WAIT: Duration = Duration::from_secs(5);

/// Appends lines to the service log on a thread of its own, so a slow disk
/// never stalls the loop that raised them, rotating the log at
/// [`SERVICE_LOG_MAX_BYTES`]. Writing is best effort: a failed write loses
/// the line, never the notice. The thread ends with the mirror.
pub struct NoticeMirror {
    /// Where lines go to the thread.
    sender: Sender<MirrorWrite>,
}

impl NoticeMirror {
    /// Starts the thread appending to `log`, keeping this process's stderr on
    /// the live log across rotations when it was redirected there.
    pub fn spawn(log: PathBuf) -> Self {
        Self::following(log, Box::new(io::stderr()))
    }

    /// Starts the thread appending to `log`, pointing `stream` at each fresh
    /// log when it wrote to the one rotated out.
    pub(super) fn following(log: PathBuf, mut stream: Box<dyn FollowedStream>) -> Self {
        let (sender, writes) = unbounded();
        thread::spawn(move || write_all(&log, &writes, stream.as_mut()));
        Self { sender }
    }

    /// Appends `line`, which ends with its newline.
    pub fn append(&self, line: Vec<u8>) {
        let _ = self.sender.send(MirrorWrite::Line(line));
    }

    /// Waits, at most [`FLUSH_WAIT`], until every line appended before it
    /// was written.
    pub fn flush(&self) {
        let (done, written) = bounded(1);
        if self.sender.send(MirrorWrite::Flush(done)).is_ok() {
            let _ = written.recv_timeout(FLUSH_WAIT);
        }
    }
}

/// The name `log` is rotated to.
fn rotated_log(log: &Path) -> PathBuf {
    let mut name = OsString::from(log.as_os_str());
    name.push(ROTATED_SUFFIX);
    PathBuf::from(name)
}

/// The mirror thread's open log: one append handle and how many bytes the
/// log holds, so a line costs one write rather than a stat and an open.
struct LogAppender {
    /// The live log, or `None` until an open succeeds.
    file: Option<File>,
    /// The log's size when opened plus every byte appended since.
    written: u64,
}

impl LogAppender {
    /// The appender for `log`, opened now when it can be.
    fn open(log: &Path) -> Self {
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
    fn append(&mut self, log: &Path, line: &[u8], stream: &mut dyn FollowedStream) {
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

/// Writes each line from `writes` to `log` in order, rotating it first when
/// the line would carry it past [`SERVICE_LOG_MAX_BYTES`].
fn write_all(log: &Path, writes: &Receiver<MirrorWrite>, stream: &mut dyn FollowedStream) {
    let mut appender = LogAppender::open(log);
    for write in writes {
        match write {
            MirrorWrite::Line(line) => appender.append(log, &line, stream),
            MirrorWrite::Flush(done) => {
                let _ = done.send(());
            },
        }
    }
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
fn open_log(log: &Path) -> io::Result<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .mode(LOG_FILE_MODE)
        .open(log)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use tempfile::TempDir;

    use super::*;
    use crate::adapter::service::shared_fd::SharedFd;

    /// A rotation recreates the log private to its owner and points a stream
    /// that wrote to the old log at the fresh one, so a later stderr line
    /// lands in the live log rather than the rotated or unlinked file.
    #[test]
    fn a_rotation_keeps_the_log_private_and_moves_the_stream_along() {
        let scratch = TempDir::new().unwrap();
        let log = scratch.path().join("service.log");
        let stream = SharedFd::new(open_log(&log).unwrap());
        let mirror = NoticeMirror::following(log.clone(), Box::new(stream.clone()));
        let line = [vec![b'x'; 4096], b"\n".to_vec()].concat();
        let lines = usize::try_from(SERVICE_LOG_MAX_BYTES).unwrap() / line.len() + 2;

        for _ in 0..lines * 2 {
            mirror.append(line.clone());
        }
        mirror.flush();
        stream.write(b"after the rotation\n");

        let mode = fs::metadata(&log).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, LOG_FILE_MODE);
        let live = fs::read_to_string(&log).unwrap();
        assert!(live.ends_with("after the rotation\n"), "{}", live.len());
        let rotated = fs::read_to_string(rotated_log(&log)).unwrap();
        assert!(!rotated.contains("after the rotation"));
    }

    /// Lines past the cap move the log to its rotated name and start a fresh
    /// one, so the log never grows without bound.
    #[test]
    fn the_log_rotates_past_its_cap() {
        let scratch = TempDir::new().unwrap();
        let log = scratch.path().join("service.log");
        let mirror = NoticeMirror::spawn(log.clone());
        let line = vec![b'x'; 4096];
        let lines = usize::try_from(SERVICE_LOG_MAX_BYTES).unwrap() / line.len() + 2;

        for _ in 0..lines {
            mirror.append([line.as_slice(), b"\n"].concat());
        }
        mirror.flush();

        let live = fs::metadata(&log).unwrap().len();
        let rotated = fs::metadata(rotated_log(&log)).unwrap().len();
        assert!(live <= SERVICE_LOG_MAX_BYTES, "{live}");
        assert!(rotated <= SERVICE_LOG_MAX_BYTES, "{rotated}");
        assert_eq!(
            live + rotated,
            (lines * (line.len() + 1)) as u64,
            "no line is lost"
        );
    }
}
