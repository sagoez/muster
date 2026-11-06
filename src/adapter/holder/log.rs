use std::{
    borrow::Cow,
    ffi::OsString,
    fs::{self, File, OpenOptions, Permissions},
    io::{self, Write},
    os::unix::fs::{FileExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use super::{generation::Generation, log_base::LogBase};
use crate::{
    adapter::{
        private_dir::create_private_dir_all,
        terminal::escape::{END_OPEN_LINE, LINE_FEED, RUN_START},
    },
    domain::value::{ByteSize, OutputCursor},
};

/// Suffix of the single retained older log generation.
const ROTATED_SUFFIX: &str = ".1";
/// How far past its cap a live file grows while a line is open, waiting for
/// the line feed to rotate after, before the log ends the line itself.
pub const MAX_OPEN_LINE_OVERRUN: u64 = 64 * 1024;
/// Mode of a log and its sidecar: its owner's alone.
pub(super) const PRIVATE_LOG_MODE: u32 = 0o600;

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
    /// Where the run being logged began: where [`Self::start_run`] was
    /// called, or where the log ended when opened.
    run_start: OutputCursor,
    /// Whether the live file ends on a line boundary: it is empty or its
    /// last byte is a line feed.
    ends_line: bool,
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
        let ends_line = ends_line(&path, live.len());
        let log = Self {
            path,
            file,
            written: live.len(),
            base,
            cap,
            run_start: OutputCursor::new(base.saturating_add(live.len())),
            ends_line,
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

    /// Where the run being logged began.
    pub fn run_start(&self) -> OutputCursor {
        self.run_start
    }

    /// Begins a run at the log's end, before the child writes anything:
    /// appends [`RUN_START`], which ends the line the last run left open
    /// and resets the terminal, so every reader renders the run from a fresh
    /// terminal without knowing where it began. Like any write, a failed one
    /// never stops the run.
    pub fn start_run(&mut self) {
        self.run_start = self.end();
        let _ = self.append(RUN_START);
    }

    /// Appends `bytes` as [`Self::append`] does, and returns them as the log
    /// holds them, after the [`END_OPEN_LINE`] it wrote first when it ended
    /// the open line, with the outcome of the write; so a caller that hands
    /// the logged bytes on hands on what the log holds.
    pub fn append_held<'a>(&mut self, bytes: Cow<'a, [u8]>) -> (Cow<'a, [u8]>, io::Result<()>) {
        let ended = self.ends_line_first(&bytes);
        let outcome = self.append(&bytes);
        if ended {
            return (Cow::Owned([END_OPEN_LINE, &bytes].concat()), outcome);
        }
        (bytes, outcome)
    }

    /// Whether appending `bytes` ends the open line first, writing
    /// [`END_OPEN_LINE`] ahead of them: a line is open, `bytes` hold no line
    /// feed to rotate after, and they would carry the live file more than
    /// [`MAX_OPEN_LINE_OVERRUN`] past the cap.
    fn ends_line_first(&self, bytes: &[u8]) -> bool {
        let limit = self.cap.bytes().saturating_add(MAX_OPEN_LINE_OVERRUN);
        !self.ends_line
            && self.written.saturating_add(bytes.len() as u64) > limit
            && !bytes.contains(&LINE_FEED)
    }

    /// Appends `bytes`, rotating only on a line boundary once they would
    /// carry a non-empty live file past the cap: before them when the live
    /// file ends a line, and otherwise after the line feed that ends its
    /// open line. A line still open runs past the cap up to
    /// [`MAX_OPEN_LINE_OVERRUN`], and is then ended with [`END_OPEN_LINE`]
    /// before the rotation: so every generation starts on a line, and an
    /// endless line still rotates.
    /// A single write larger than the cap is kept whole.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when rotating or writing fails.
    pub fn append(&mut self, bytes: &[u8]) -> io::Result<()> {
        if self.ends_line_first(bytes) {
            self.write(END_OPEN_LINE)?;
            self.rotate()?;
            return self.write(bytes);
        }
        let incoming = bytes.len() as u64;
        if self.written == 0 || self.written.saturating_add(incoming) <= self.cap.bytes() {
            return self.write(bytes);
        }
        let head = if self.ends_line {
            0
        } else {
            match bytes.iter().position(|byte| *byte == LINE_FEED) {
                Some(feed) => feed + 1,
                None => return self.write(bytes),
            }
        };
        self.write(&bytes[..head])?;
        self.rotate()?;
        self.write(&bytes[head..])
    }

    /// Writes `bytes` to the live file. After a failed write, which may have
    /// written part of them, the length and whether the file ends a line are
    /// taken from the file again, so [`Self::end`] keeps naming real log
    /// positions and rotation sees the line as it is.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when writing fails.
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        if let Err(error) = self.file.write_all(bytes) {
            if let Ok(metadata) = self.file.metadata() {
                self.written = metadata.len();
            }
            self.ends_line = ends_line(&self.path, self.written);
            return Err(error);
        }
        self.written = self.written.saturating_add(bytes.len() as u64);
        if let Some(last) = bytes.last() {
            self.ends_line = *last == LINE_FEED;
        }
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
        self.ends_line = true;
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

/// Whether the file at `path`, `len` bytes long, ends on a line boundary:
/// it is empty or its last byte is a line feed. A last byte that cannot be
/// read counts as leaving a line open, which only makes rotation wait for a
/// line feed.
fn ends_line(path: &Path, len: u64) -> bool {
    let Some(last) = len.checked_sub(1) else {
        return true;
    };
    let mut byte = [0];
    File::open(path)
        .and_then(|file| file.read_exact_at(&mut byte, last))
        .is_ok_and(|()| byte[0] == LINE_FEED)
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
        log.append(b"123\n").unwrap();
        log.append(b"567\n").unwrap();

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
        log.append(b"1234\n").unwrap();
        log.append(b"67\n").unwrap();
        log.append(b"abcde\n").unwrap();

        assert_eq!(fs::read(rotated_path(&path)).unwrap(), b"1234\n67\n");
        assert_eq!(fs::read(&path).unwrap(), b"abcde\n");

        drop(log);
        let mut reopened = RotatingLog::open(path.clone(), cap).unwrap();
        reopened.append(b"ghij\n").unwrap();
        assert_eq!(
            fs::read(rotated_path(&path)).unwrap(),
            b"abcde\n",
            "a reopened log counts its existing bytes and keeps one generation"
        );
        assert_eq!(fs::read(&path).unwrap(), b"ghij\n");
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
        log.append(b"1234567\n").unwrap();
        log.append(b"abcde\n").unwrap();
        assert_eq!(log.end(), OutputCursor::new(14));
        assert_eq!(LogBase::read(&path).unwrap().offset(), 8);

        drop(log);
        let mut reopened = RotatingLog::open(path.clone(), cap).unwrap();
        assert_eq!(reopened.end(), OutputCursor::new(14), "a new run continues");
        reopened.append(b"ghij\n").unwrap();
        assert_eq!(reopened.end(), OutputCursor::new(19));
        assert_eq!(LogBase::read(&path).unwrap().offset(), 14);
        assert_eq!(fs::read(rotated_path(&path)).unwrap(), b"abcde\n");
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

    /// A write that carries the log past its cap while a line is open
    /// rotates after the line feed that ends it, even in a log reopened
    /// mid-line, so the new generation starts on a line and a reader that
    /// starts at the oldest byte kept never starts inside a sequence; one
    /// that finds a line ended rotates before it.
    #[test]
    fn rotation_starts_the_new_generation_on_a_line() {
        let scratch = TempDir::new().unwrap();
        let path = scratch.path().join("proc.log");
        let cap = ByteSize::try_new(RawByteSize::b(10)).unwrap();
        let mut log = RotatingLog::open(path.clone(), cap).unwrap();
        log.append(b"12345678").unwrap();
        drop(log);
        let mut log = RotatingLog::open(path.clone(), cap).unwrap();
        log.append(b"9\n\x1b[31mred\nmore").unwrap();

        assert_eq!(fs::read(rotated_path(&path)).unwrap(), b"123456789\n");
        assert_eq!(fs::read(&path).unwrap(), b"\x1b[31mred\nmore");
        assert_eq!(LogBase::read(&path).unwrap().offset(), 10);
        assert_eq!(log.end(), OutputCursor::new(23));

        log.append(b" done\n").unwrap();
        log.append(b"next\nline\n").unwrap();
        assert_eq!(
            fs::read(rotated_path(&path)).unwrap(),
            b"\x1b[31mred\nmore done\n"
        );
        assert_eq!(fs::read(&path).unwrap(), b"next\nline\n");
    }

    /// Writes with no line feed that carry an open line across the cap
    /// never rotate inside it: the line runs on until its line feed, and the
    /// generation after starts on the next line. An endless line ends where
    /// it passes [`MAX_OPEN_LINE_OVERRUN`] beyond the cap, with
    /// [`END_OPEN_LINE`] closing it in the older generation, so the new one
    /// still starts on a line.
    #[test]
    fn a_long_line_across_the_cap_never_splits_a_generation() {
        let scratch = TempDir::new().unwrap();
        let path = scratch.path().join("proc.log");
        let cap = ByteSize::try_new(RawByteSize::b(10)).unwrap();
        let mut log = RotatingLog::open(path.clone(), cap).unwrap();
        log.append(b"\x1b[31mlong").unwrap();
        log.append(b" line \x1b[").unwrap();
        log.append(b"0m going on").unwrap();
        assert!(!rotated_path(&path).exists(), "no rotation inside the line");

        log.append(b" ends\nnext\n").unwrap();
        assert_eq!(
            fs::read(rotated_path(&path)).unwrap(),
            b"\x1b[31mlong line \x1b[0m going on ends\n"
        );
        assert_eq!(fs::read(&path).unwrap(), b"next\n");

        let endless = vec![b'x'; usize::try_from(MAX_OPEN_LINE_OVERRUN + cap.bytes()).unwrap()];
        log.append(b"open").unwrap();
        assert!(!log.ends_line_first(&endless[..1024]));
        assert!(log.ends_line_first(&endless));
        let before = log.end();
        let (held, outcome) = log.append_held(Cow::Borrowed(&endless));
        outcome.unwrap();

        assert_eq!(held, [END_OPEN_LINE, &endless].concat());
        assert_eq!(
            fs::read(rotated_path(&path)).unwrap(),
            [b"next\nopen".as_slice(), END_OPEN_LINE].concat()
        );
        assert_eq!(fs::read(&path).unwrap(), endless);
        assert_eq!(
            log.end(),
            before.advanced((END_OPEN_LINE.len() + endless.len()) as u64)
        );
        assert_eq!(
            LogBase::read(&path).unwrap().offset(),
            before.advanced(END_OPEN_LINE.len() as u64).offset()
        );
    }

    /// A log whose last byte cannot be read opens all the same, taken as
    /// leaving a line open.
    #[test]
    fn an_unreadable_last_byte_reads_as_an_open_line() {
        let scratch = TempDir::new().unwrap();
        let path = scratch.path().join("proc.log");
        fs::write(&path, b"line\n").unwrap();

        assert!(ends_line(&path, 5));
        assert!(!ends_line(&path, 50), "past the end");
        assert!(!ends_line(&scratch.path().join("gone.log"), 5));
    }

    /// A write that fails after another writer left the file mid-line
    /// leaves the log knowing the line is open, so it never rotates inside
    /// it.
    #[test]
    fn a_failed_write_takes_the_line_state_from_the_file() {
        let scratch = TempDir::new().unwrap();
        let path = scratch.path().join("proc.log");
        let cap = ByteSize::try_new(RawByteSize::mib(1)).unwrap();
        let mut log = RotatingLog::open(path.clone(), cap).unwrap();
        log.append(b"line\n").unwrap();
        RotatingLog::append_to(&path)
            .unwrap()
            .write_all(b"part")
            .unwrap();
        log.file = File::open(&path).unwrap();

        assert!(log.append(b"rest of it\n").is_err());

        assert_eq!(log.end(), OutputCursor::new(9));
        assert!(!log.ends_line, "the file ends inside a line");
    }

    /// Each run begins with the reset every reader renders it from, in the
    /// log itself, and its start is where that reset was written.
    #[test]
    fn each_run_starts_with_a_reset_in_the_log() {
        let scratch = TempDir::new().unwrap();
        let path = scratch.path().join("proc.log");
        let cap = ByteSize::try_new(RawByteSize::mib(1)).unwrap();
        let mut log = RotatingLog::open(path.clone(), cap).unwrap();
        log.append(b"before").unwrap();
        assert_eq!(log.run_start(), OutputCursor::new(0));

        log.start_run();
        log.append(b"run\n").unwrap();

        assert_eq!(log.run_start(), OutputCursor::new(6));
        assert_eq!(
            fs::read(&path).unwrap(),
            [b"before".as_slice(), RUN_START, b"run\n"].concat()
        );
        drop(log);
        let reopened = RotatingLog::open(path.clone(), cap).unwrap();
        assert_eq!(reopened.run_start(), reopened.end());
    }
}
