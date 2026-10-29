use std::{
    io,
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

use crossbeam_channel::{Receiver, Sender, bounded, unbounded};

use super::{
    followed_stream::FollowedStream, log_appender::LogAppender, mirror_write::MirrorWrite,
};
/// Longest [`NoticeMirror::flush`] waits for the lines before it.
pub(super) const NOTICE_FLUSH_WAIT: Duration = Duration::from_secs(5);

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

    /// Waits, at most [`NOTICE_FLUSH_WAIT`], until every line appended before
    /// it was written.
    pub fn flush(&self) {
        let (done, written) = bounded(1);
        if self.sender.send(MirrorWrite::Flush(done)).is_ok() {
            let _ = written.recv_timeout(NOTICE_FLUSH_WAIT);
        }
    }
}

/// Writes each line from `writes` to `log` in order, rotating it first when
/// the line would carry it past [`SERVICE_LOG_MAX_BYTES`](super::log_appender::SERVICE_LOG_MAX_BYTES).
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

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::PermissionsExt};

    use tempfile::TempDir;

    use super::*;
    use crate::adapter::service::{
        log_appender::{SERVICE_LOG_MAX_BYTES, open_log, rotated_log},
        shared_fd::SharedFd,
        state_dir::LOG_FILE_MODE,
    };

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
