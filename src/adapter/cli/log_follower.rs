use std::{
    fs::{self, File, Metadata},
    io::{self, Seek, SeekFrom},
    os::unix::fs::MetadataExt,
    path::PathBuf,
};

use super::{file_identity::FileIdentity, followed_output::FollowedOutput};
use crate::{
    adapter::holder::{live_start, rotated_path},
    domain::value::OutputCursor,
};

/// Follows a process log the way `tail -f` does, across the holder's
/// rotation, by output cursor: when the live path starts naming a new file,
/// the old one is read to its end, then the new one from the first byte not
/// yet written. Output a rotation discarded before it was read is reported
/// as a gap naming the bytes lost, never skipped silently.
pub struct LogFollower {
    /// The live log path.
    path: PathBuf,
    /// The file being read and which file it is.
    file: Option<(File, FileIdentity)>,
    /// Where the next byte written sits in the process's output.
    next: OutputCursor,
}

impl LogFollower {
    /// A follower of `path` that has written the output before `next`.
    pub fn new(path: PathBuf, next: OutputCursor) -> Self {
        Self {
            path,
            file: None,
            next,
        }
    }

    /// Copies everything written to the log since the last call into `out`
    /// and returns how many bytes that was.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when the log cannot be read or `out` cannot
    /// be written. A live path that is briefly missing mid-rotation is not an
    /// error.
    pub fn pump(&mut self, out: &mut dyn FollowedOutput) -> io::Result<u64> {
        let mut copied = self.drain(out)?;
        let current = match fs::metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(copied),
            Err(error) => return Err(error),
        };
        if let Some((file, followed)) = &mut self.file
            && *followed == identity(&current)
        {
            if current.len() < file.stream_position()? {
                file.seek(SeekFrom::Start(0))?;
                copied += self.drain(out)?;
            }
            return Ok(copied);
        }
        copied += self.drain(out)?;
        let mut file = File::open(&self.path)?;
        let opened = identity(&file.metadata()?);
        let start = live_start(&self.path, &file)?;
        copied += self.catch_up(start, out)?;
        file.seek(SeekFrom::Start(self.next.bytes_since(start)))?;
        self.file = Some((file, opened));
        copied += self.drain(out)?;
        Ok(copied)
    }

    /// Brings the follower up to `start`, where the live file begins, when
    /// more than one rotation passed since it last read: what the rotated
    /// file still holds is copied, and what no file holds any more is
    /// reported as a gap.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when the rotated file cannot be read or `out`
    /// cannot be written.
    fn catch_up(&mut self, start: OutputCursor, out: &mut dyn FollowedOutput) -> io::Result<u64> {
        if start <= self.next {
            return Ok(0);
        }
        let mut copied = 0;
        match File::open(rotated_path(&self.path)) {
            Ok(mut rotated) => {
                let len = rotated.metadata()?.len();
                let rotated_start = OutputCursor::new(start.offset().saturating_sub(len));
                self.skip_to(rotated_start, out)?;
                rotated.seek(SeekFrom::Start(self.next.bytes_since(rotated_start)))?;
                copied = io::copy(&mut rotated, out)?;
                self.next = self.next.advanced(copied);
            },
            Err(error) if error.kind() == io::ErrorKind::NotFound => {},
            Err(error) => return Err(error),
        }
        self.skip_to(start, out)?;
        Ok(copied)
    }

    /// Reports the output from here to `to` as lost, when `to` is ahead.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when `out` cannot be written.
    fn skip_to(&mut self, to: OutputCursor, out: &mut dyn FollowedOutput) -> io::Result<()> {
        if to > self.next {
            out.gap(to.bytes_since(self.next), to)?;
            self.next = to;
        }
        Ok(())
    }

    /// Copies what remains of the followed file into `out`.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when the copy fails.
    fn drain(&mut self, out: &mut dyn FollowedOutput) -> io::Result<u64> {
        let copied = match &mut self.file {
            Some((file, _)) => io::copy(file, out)?,
            None => 0,
        };
        self.next = self.next.advanced(copied);
        Ok(copied)
    }
}

/// The device and inode `metadata` describes.
fn identity(metadata: &Metadata) -> FileIdentity {
    (metadata.dev(), metadata.ino())
}

#[cfg(test)]
mod tests {
    use std::{fs::OpenOptions, io::Write, path::Path};

    use bytesize::ByteSize as RawByteSize;
    use tempfile::TempDir;

    use super::*;
    use crate::{
        adapter::{
            cli::rendered_log::{RenderedLog, gap_line},
            holder::RotatingLog,
            terminal::colouring::Colouring,
        },
        domain::value::ByteSize,
    };

    /// The cap of the logs these tests rotate.
    const CAP_BYTES: u64 = 8;

    /// Appends `text` to the file at `path`.
    fn append(path: &Path, text: &str) {
        OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
    }

    /// What one pump of `follower` copies, as text.
    fn pump(follower: &mut LogFollower) -> String {
        let mut bytes = Vec::new();
        follower
            .pump(&mut RenderedLog::new(&mut bytes, Colouring::Plain))
            .unwrap();
        String::from_utf8(bytes).unwrap()
    }

    /// A log at `path` that rotates past [`CAP_BYTES`], as a holder writes it.
    fn rotating(path: &Path) -> RotatingLog {
        RotatingLog::open(
            path.to_path_buf(),
            ByteSize::try_new(RawByteSize::b(CAP_BYTES)).unwrap(),
        )
        .unwrap()
    }

    /// Lines written before, across and after a rotation each come out once
    /// and in order, including the old file's last lines written after the
    /// follower last read it.
    #[test]
    fn logs_follow_survives_rotation() {
        let scratch = TempDir::new().unwrap();
        let path = scratch.path().join("api.log");
        let mut log = rotating(&path);
        log.append(b"one\n").unwrap();
        let mut follower = LogFollower::new(path.clone(), OutputCursor::new(0));

        assert_eq!(pump(&mut follower), "one\n");
        assert_eq!(pump(&mut follower), "", "nothing new, nothing repeated");

        log.append(b"two\n").unwrap();
        log.append(b"three\n").unwrap();
        assert_eq!(
            pump(&mut follower),
            "two\nthree\n",
            "the old file's tail comes before the new file"
        );
        log.append(b"four\n").unwrap();
        assert_eq!(pump(&mut follower), "four\n");
        assert_eq!(follower.next, log.end());
    }

    /// Two rotations between reads lose the middle file; the follower says
    /// how many bytes went, then carries on with the live file.
    #[test]
    fn a_missed_rotation_is_reported_as_a_gap() {
        let scratch = TempDir::new().unwrap();
        let path = scratch.path().join("api.log");
        let mut log = rotating(&path);
        log.append(b"aaaaaa\n").unwrap();
        let mut follower = LogFollower::new(path.clone(), OutputCursor::new(0));
        assert_eq!(pump(&mut follower), "aaaaaa\n");

        log.append(b"bbbbbb\n").unwrap();
        log.append(b"cccccc\n").unwrap();
        log.append(b"dddddd\n").unwrap();
        fs::remove_file(rotated_path(&path)).unwrap();

        assert_eq!(
            pump(&mut follower),
            format!("{}\ndddddd\n", gap_line(14)),
            "the two lines rotated away unread are named, the live one follows"
        );
    }

    /// When the rotated file still holds part of what was missed, that part
    /// is read and only the rest is a gap.
    #[test]
    fn a_missed_rotation_reads_what_the_rotated_file_still_holds() {
        let scratch = TempDir::new().unwrap();
        let path = scratch.path().join("api.log");
        let mut log = rotating(&path);
        log.append(b"aaaaaa\n").unwrap();
        let mut follower = LogFollower::new(path.clone(), OutputCursor::new(0));
        assert_eq!(pump(&mut follower), "aaaaaa\n");

        log.append(b"bbbbbb\n").unwrap();
        log.append(b"cccccc\n").unwrap();
        log.append(b"dddddd\n").unwrap();

        assert_eq!(
            pump(&mut follower),
            format!("{}\ncccccc\ndddddd\n", gap_line(7))
        );
        assert_eq!(follower.next, log.end());
    }

    /// Output a reader already wrote, before the follower takes over, is not
    /// written again.
    #[test]
    fn output_already_written_is_not_repeated() {
        let scratch = TempDir::new().unwrap();
        let path = scratch.path().join("api.log");
        let mut log = rotating(&path);
        log.append(b"seen\n").unwrap();
        let mut follower = LogFollower::new(path.clone(), log.end());
        log.append(b"new\n").unwrap();

        assert_eq!(pump(&mut follower), "new\n");
    }

    #[test]
    fn a_log_not_yet_written_is_followed_once_it_appears() {
        let scratch = TempDir::new().unwrap();
        let log = scratch.path().join("late.log");
        let mut follower = LogFollower::new(log.clone(), OutputCursor::new(0));

        assert_eq!(pump(&mut follower), "");
        append(&log, "hello\n");
        assert_eq!(pump(&mut follower), "hello\n");
    }

    #[test]
    fn a_truncated_log_is_read_again_from_its_start() {
        let scratch = TempDir::new().unwrap();
        let log = scratch.path().join("api.log");
        append(&log, "a long first line\n");
        let mut follower = LogFollower::new(log.clone(), OutputCursor::new(0));
        pump(&mut follower);

        fs::write(&log, "new\n").unwrap();

        assert_eq!(pump(&mut follower), "new\n");
    }
}
