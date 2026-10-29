use std::{
    ffi::OsString,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use crossbeam_channel::{Receiver, Sender, bounded, unbounded};

use super::mirror_write::MirrorWrite;

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
    /// Starts the thread appending to `log`.
    pub fn spawn(log: PathBuf) -> Self {
        let (sender, writes) = unbounded();
        thread::spawn(move || write_all(&log, &writes));
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

/// Writes each line from `writes` to `log` in order, rotating it first when
/// the line would carry it past [`SERVICE_LOG_MAX_BYTES`].
fn write_all(log: &Path, writes: &Receiver<MirrorWrite>) {
    for write in writes {
        let line = match write {
            MirrorWrite::Line(line) => line,
            MirrorWrite::Flush(done) => {
                let _ = done.send(());
                continue;
            },
        };
        let written = fs::metadata(log).map_or(0, |metadata| metadata.len());
        if written > 0 && written.saturating_add(line.len() as u64) > SERVICE_LOG_MAX_BYTES {
            let _ = fs::rename(log, rotated_log(log));
        }
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(log) {
            let _ = file.write_all(&line);
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

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
