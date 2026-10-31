use std::{
    ffi::OsString,
    fs::{self, File, OpenOptions, Permissions},
    io::{self, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

use super::{
    generation::Generation,
    log_base::LogBase,
    mark_file::{MAX_STREAM_MARKS, append_marks, stream_marks, write_marks},
};
use crate::{
    adapter::{
        private_dir::create_private_dir_all,
        terminal::{
            mark_kind::MarkKind, mode_scanner::ModeScanner, observation::Observation,
            stream_mark::StreamMark, stream_marks::StreamMarks,
        },
    },
    domain::value::{ByteSize, OutputCursor},
};

/// Suffix of the single retained older log generation.
const ROTATED_SUFFIX: &str = ".1";
/// Mode of a log and its sidecar: its owner's alone.
pub(super) const PRIVATE_LOG_MODE: u32 = 0o600;

/// An append-only log that keeps one older generation: once the next write
/// would pass the cap, `name.log` becomes `name.log.1` and a fresh file starts.
/// It also tracks where each byte sits in the process's output stream across
/// rotations and runs, recording the live file's start in a `.base` sidecar,
/// and where each run starts and the alternate screen comes up or goes away
/// in a `.marks` one. The marks only help readers render, so failing to
/// write them never fails a run or a write: the failure is kept for
/// [`RotatingLog::take_warning`].
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
    /// Follows the alternate screen through what is appended.
    scanner: ModeScanner,
    /// Whether the alternate screen is up at the log's end.
    alternate: bool,
    /// How many marks the sidecar holds.
    marks: usize,
    /// Why the marks sidecar could not be written, until it is reported.
    warning: Option<String>,
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
        let marks = stream_marks(&path).marks().len();
        let log = Self {
            path,
            file,
            written: live.len(),
            base,
            cap,
            scanner: ModeScanner::new(),
            alternate: false,
            marks,
            warning: None,
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

    /// Records that a run begins at the log's end, before it writes anything,
    /// so a reader that reads the run's output and then the marks always
    /// finds its start; the run renders from a fresh terminal. Marks past
    /// the end, left by a log reopened shorter, are dropped, as no reader
    /// can follow them.
    pub fn record_run_start(&mut self) {
        self.scanner = ModeScanner::new();
        self.alternate = false;
        let end = self.end();
        let start = StreamMark::builder()
            .at(end)
            .kind(MarkKind::RunStart)
            .build();
        let marks = stream_marks(&self.path);
        if marks.marks().last().is_none_or(|mark| mark.at() <= end) {
            self.add_marks(&[start]);
            return;
        }
        let kept = marks
            .marks()
            .iter()
            .copied()
            .filter(|mark| mark.at() <= end)
            .chain([start])
            .collect();
        self.replace_marks(&StreamMarks::new(kept));
    }

    /// Why the marks sidecar could not be written since the last call, if
    /// it could not.
    pub fn take_warning(&mut self) -> Option<String> {
        self.warning.take()
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
        let at = self.end();
        if let Err(error) = self.file.write_all(bytes) {
            if let Ok(metadata) = self.file.metadata() {
                self.written = metadata.len();
            }
            return Err(error);
        }
        self.written = self.written.saturating_add(incoming);
        self.mark_screens(at, bytes);
        Ok(())
    }

    /// Records where `bytes`, written at `at`, bring the alternate screen up
    /// or take it away, each just past the sequence that does it.
    fn mark_screens(&mut self, at: OutputCursor, bytes: &[u8]) {
        let mut found = Vec::new();
        for (index, byte) in bytes.iter().enumerate() {
            let up = match self.scanner.feed(*byte) {
                Observation::EnterAlternate => true,
                Observation::LeaveAlternate | Observation::FullReset => false,
                _ => continue,
            };
            if up == self.alternate {
                continue;
            }
            self.alternate = up;
            let kind = if up {
                MarkKind::EnterAlternate
            } else {
                MarkKind::LeaveAlternate
            };
            found.push(
                StreamMark::builder()
                    .at(at.advanced(index as u64 + 1))
                    .kind(kind)
                    .build(),
            );
        }
        if !found.is_empty() {
            self.add_marks(&found);
        }
    }

    /// Appends `marks` to the sidecar, bounding it to [`MAX_STREAM_MARKS`]
    /// once it holds more.
    fn add_marks(&mut self, marks: &[StreamMark]) {
        if let Err(error) = append_marks(&self.path, marks) {
            self.warn(&error);
            return;
        }
        self.marks += marks.len();
        if self.marks > MAX_STREAM_MARKS {
            self.retain_marks(OutputCursor::new(0));
        }
    }

    /// Bounds the sidecar to the marks a log whose oldest byte is `oldest`
    /// still needs, and to [`MAX_STREAM_MARKS`].
    fn retain_marks(&mut self, oldest: OutputCursor) {
        let kept = stream_marks(&self.path).retained(oldest, MAX_STREAM_MARKS);
        self.replace_marks(&kept);
    }

    /// Replaces the sidecar with `marks`.
    fn replace_marks(&mut self, marks: &StreamMarks) {
        match write_marks(&self.path, marks) {
            Ok(()) => self.marks = marks.marks().len(),
            Err(error) => self.warn(&error),
        }
    }

    /// Keeps `error`, met writing the marks sidecar, to be reported.
    fn warn(&mut self, error: &io::Error) {
        self.warning = Some(format!(
            "could not record run starts and screen changes beside {}: {error}; \
             reads of this log may render a run as the one before it left the terminal",
            self.path.display()
        ));
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
        let oldest = OutputCursor::new(self.base);
        self.base = self.base.saturating_add(self.written);
        self.written = 0;
        self.record_base()?;
        self.retain_marks(oldest);
        Ok(())
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
    use crate::adapter::holder::span::Span;

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

    /// A mark of `kind` at `offset`.
    fn mark(kind: MarkKind, offset: u64) -> StreamMark {
        StreamMark::builder()
            .at(OutputCursor::new(offset))
            .kind(kind)
            .build()
    }

    /// Each run start is marked, and the alternate screen is marked just past
    /// the sequence that brings it up or takes it away, once per change, a
    /// full reset taking it away too, even when a sequence is split.
    #[test]
    fn runs_and_screen_changes_are_marked() {
        let scratch = TempDir::new().unwrap();
        let path = scratch.path().join("proc.log");
        let mut log = RotatingLog::open(
            path.clone(),
            ByteSize::try_new(RawByteSize::mib(1)).unwrap(),
        )
        .unwrap();

        log.record_run_start();
        log.append(b"ab\x1b[?10").unwrap();
        log.append(b"49h\x1b[?1049hTUI\x1b[?1049l").unwrap();
        log.append(b"\x1b[?47h\x1bc").unwrap();
        log.record_run_start();

        assert_eq!(stream_marks(&path).marks(), [
            mark(MarkKind::RunStart, 0),
            mark(MarkKind::EnterAlternate, 10),
            mark(MarkKind::LeaveAlternate, 29),
            mark(MarkKind::EnterAlternate, 35),
            mark(MarkKind::LeaveAlternate, 37),
            mark(MarkKind::RunStart, 37),
        ]);
        assert_eq!(log.take_warning(), None);
    }

    /// A marks sidecar that cannot be written never stops a run or a write:
    /// the log goes on, and why the marks are missing is kept to report.
    #[test]
    fn an_unwritable_marks_sidecar_never_stops_the_log() {
        let scratch = TempDir::new().unwrap();
        let path = scratch.path().join("proc.log");
        let mut marks = path.clone().into_os_string();
        marks.push(".marks");
        fs::create_dir(&marks).unwrap();
        let mut log = RotatingLog::open(
            path.clone(),
            ByteSize::try_new(RawByteSize::mib(1)).unwrap(),
        )
        .unwrap();

        log.record_run_start();
        let warning = log.take_warning().expect("the failure is kept");
        log.append(b"ready\n\x1b[?1049h").unwrap();

        assert!(warning.contains("proc.log"), "{warning}");
        assert!(log.take_warning().is_some(), "a later failure is kept too");
        assert_eq!(fs::read(&path).unwrap(), b"ready\n\x1b[?1049h");
    }

    /// Runs whose bytes rotation discarded lose their marks at the rotation,
    /// but for those the screen at the oldest byte kept comes from, so the
    /// sidecar follows the log's size rather than the number of runs.
    #[test]
    fn rotation_drops_the_marks_of_discarded_output() {
        let scratch = TempDir::new().unwrap();
        let path = scratch.path().join("proc.log");
        let cap = ByteSize::try_new(RawByteSize::b(16)).unwrap();
        let mut log = RotatingLog::open(path.clone(), cap).unwrap();
        for _ in 0..100 {
            log.record_run_start();
            log.append(b"run\n").unwrap();
        }
        log.append(b"\x1b[?1049h").unwrap();
        for _ in 0..10 {
            log.append(b"frame\n").unwrap();
        }

        let marks = stream_marks(&path);
        let oldest = Span::open(&path).unwrap().oldest();

        assert!(marks.marks().len() <= 4, "{:?}", marks.marks());
        assert!(marks.alternate_at(oldest));
        assert_eq!(log.take_warning(), None);
    }
}
