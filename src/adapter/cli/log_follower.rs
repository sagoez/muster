use std::{
    fs::{self, File, Metadata},
    io::{self, Seek, SeekFrom, Write},
    os::unix::fs::MetadataExt,
    path::PathBuf,
};

use super::file_identity::FileIdentity;

/// Follows a process log the way `tail -f` does, across the holder's
/// rotation: when the live path starts naming a new file, the old one is read
/// to its end before the new one is read from its start, so nothing is
/// skipped or repeated.
pub struct LogFollower {
    /// The live log path.
    path: PathBuf,
    /// The file being read and which file it is.
    file: Option<(File, FileIdentity)>,
}

impl LogFollower {
    /// A follower of `path` that has read nothing yet; the first
    /// [`Self::pump`] copies the whole live file.
    pub fn new(path: PathBuf) -> Self {
        Self { path, file: None }
    }

    /// Copies everything written to the log since the last call into `out`
    /// and returns how many bytes that was.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when the log cannot be read or `out` cannot
    /// be written. A live path that is briefly missing mid-rotation is not an
    /// error.
    pub fn pump(&mut self, out: &mut dyn Write) -> io::Result<u64> {
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
                copied += io::copy(file, out)?;
            }
            return Ok(copied);
        }
        copied += self.drain(out)?;
        let file = File::open(&self.path)?;
        let opened = identity(&file.metadata()?);
        self.file = Some((file, opened));
        copied += self.drain(out)?;
        Ok(copied)
    }

    /// Copies what remains of the followed file into `out`.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when the copy fails.
    fn drain(&mut self, out: &mut dyn Write) -> io::Result<u64> {
        match &mut self.file {
            Some((file, _)) => io::copy(file, out),
            None => Ok(0),
        }
    }
}

/// The device and inode `metadata` describes.
fn identity(metadata: &Metadata) -> FileIdentity {
    (metadata.dev(), metadata.ino())
}

#[cfg(test)]
mod tests {
    use std::{fs::OpenOptions, path::Path};

    use tempfile::TempDir;

    use super::*;
    use crate::adapter::holder::rotated_path;

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
        let mut out = Vec::new();
        follower.pump(&mut out).unwrap();
        String::from_utf8(out).unwrap()
    }

    /// Lines written before, across and after a rotation each come out once
    /// and in order, including the old file's last lines written after the
    /// follower last read it.
    #[test]
    fn logs_follow_survives_rotation() {
        let scratch = TempDir::new().unwrap();
        let log = scratch.path().join("api.log");
        append(&log, "one\n");
        let mut follower = LogFollower::new(log.clone());

        assert_eq!(pump(&mut follower), "one\n");
        assert_eq!(pump(&mut follower), "", "nothing new, nothing repeated");

        append(&log, "two\n");
        fs::rename(&log, rotated_path(&log)).unwrap();
        assert_eq!(
            pump(&mut follower),
            "two\n",
            "mid-rotation the old file is read"
        );
        append(&log, "three\n");
        append(&log, "four\n");

        assert_eq!(pump(&mut follower), "three\nfour\n");

        append(&log, "five\n");
        fs::rename(&log, rotated_path(&log)).unwrap();
        append(&log, "six\n");
        assert_eq!(
            pump(&mut follower),
            "five\nsix\n",
            "a rotation between reads keeps the old tail before the new file"
        );
    }

    #[test]
    fn a_log_not_yet_written_is_followed_once_it_appears() {
        let scratch = TempDir::new().unwrap();
        let log = scratch.path().join("late.log");
        let mut follower = LogFollower::new(log.clone());

        assert_eq!(pump(&mut follower), "");
        append(&log, "hello\n");
        assert_eq!(pump(&mut follower), "hello\n");
    }

    #[test]
    fn a_truncated_log_is_read_again_from_its_start() {
        let scratch = TempDir::new().unwrap();
        let log = scratch.path().join("api.log");
        append(&log, "a long first line\n");
        let mut follower = LogFollower::new(log.clone());
        pump(&mut follower);

        fs::write(&log, "new\n").unwrap();

        assert_eq!(pump(&mut follower), "new\n");
    }
}
