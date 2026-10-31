//! The marks of a process's output, kept in a `.marks` sidecar beside its
//! log: where each run began, so every reader renders each run from a fresh
//! terminal, and where the alternate screen came up or went away, so a
//! reader that starts partway through a run starts on the screen the run
//! was on there. The holder appends a mark as it happens and rewrites the
//! sidecar, bounded to what the log still holds, at each rotation.

use std::{
    ffi::OsString,
    fs::{self, OpenOptions as FileOptions},
    io::{self, Write},
    os::unix::fs::OpenOptionsExt as _,
    path::{Path, PathBuf},
    time::SystemTime,
};

use atomic_write_file::{OpenOptions, unix::OpenOptionsExt as _};

use super::log::PRIVATE_LOG_MODE;
use crate::adapter::terminal::{
    escape::LINE_FEED, stream_mark::StreamMark, stream_marks::StreamMarks,
};

/// Suffix of the sidecar that records the marks.
const MARKS_SUFFIX: &str = ".marks";
/// Most marks the sidecar keeps, the newest: a backstop for a process that
/// flips screens far more often than the log's size bounds, since the marks
/// are otherwise dropped only once the log no longer holds their bytes.
pub const MAX_STREAM_MARKS: usize = 64 * 1024;

/// When and at what size the sidecar was last written, which changes
/// whenever a mark is added.
pub type MarksStamp = (u64, SystemTime);

/// The path of the sidecar beside the log at `log`.
fn marks_path(log: &Path) -> PathBuf {
    let mut marks = OsString::from(log.as_os_str());
    marks.push(MARKS_SUFFIX);
    PathBuf::from(marks)
}

/// The marks of the log at `log`; none when the sidecar is absent. A line
/// that does not parse, or the last one while its line feed is still to be
/// written, is skipped.
pub fn stream_marks(log: &Path) -> StreamMarks {
    let Ok(text) = fs::read_to_string(marks_path(log)) else {
        return StreamMarks::default();
    };
    let ended = text.rfind(char::from(LINE_FEED)).map_or(0, |end| end + 1);
    StreamMarks::new(
        text[..ended]
            .lines()
            .filter_map(|line| line.parse().ok())
            .collect(),
    )
}

/// When and at what size the sidecar of the log at `log` was last written,
/// so a reader reads it again only once it changed; none while it is absent.
pub fn marks_stamp(log: &Path) -> Option<MarksStamp> {
    let metadata = fs::metadata(marks_path(log)).ok()?;
    Some((metadata.len(), metadata.modified().ok()?))
}

/// Appends `marks` to the sidecar of the log at `log`, creating it its
/// owner's alone.
///
/// # Errors
/// Returns an [`io::Error`] when the sidecar cannot be written.
pub(super) fn append_marks(log: &Path, marks: &[StreamMark]) -> io::Result<()> {
    let text: String = marks.iter().map(|mark| format!("{mark}\n")).collect();
    FileOptions::new()
        .create(true)
        .append(true)
        .mode(PRIVATE_LOG_MODE)
        .open(marks_path(log))?
        .write_all(text.as_bytes())
}

/// Replaces the sidecar of the log at `log` with `marks`, atomically, its
/// owner's alone.
///
/// # Errors
/// Returns an [`io::Error`] when the sidecar cannot be written.
pub(super) fn write_marks(log: &Path, marks: &StreamMarks) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .preserve_mode(false)
        .mode(PRIVATE_LOG_MODE)
        .open(marks_path(log))?;
    for mark in marks.marks() {
        writeln!(file, "{mark}")?;
    }
    file.commit()
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use tempfile::TempDir;

    use super::*;
    use crate::{adapter::terminal::mark_kind::MarkKind, domain::value::OutputCursor};

    /// Mode bits a permission check compares.
    const PERMISSION_BITS: u32 = 0o777;

    /// A mark of `kind` at `offset`.
    fn mark(kind: MarkKind, offset: u64) -> StreamMark {
        StreamMark::builder()
            .at(OutputCursor::new(offset))
            .kind(kind)
            .build()
    }

    /// The permission bits of the sidecar beside `log`.
    fn mode_of(log: &Path) -> u32 {
        fs::metadata(marks_path(log)).unwrap().permissions().mode() & PERMISSION_BITS
    }

    /// Marks read back in order and private; a line that does not parse is
    /// skipped, and so is a last line still being written.
    #[test]
    fn marks_round_trip_private_and_skip_what_does_not_parse() {
        let scratch = TempDir::new().unwrap();
        let log = scratch.path().join("api.log");
        assert!(stream_marks(&log).marks().is_empty());
        assert_eq!(marks_stamp(&log), None);

        append_marks(&log, &[mark(MarkKind::RunStart, 10)]).unwrap();
        append_marks(&log, &[
            mark(MarkKind::EnterAlternate, 14),
            mark(MarkKind::RunStart, 3),
        ])
        .unwrap();

        assert_eq!(stream_marks(&log).marks(), [
            mark(MarkKind::RunStart, 3),
            mark(MarkKind::RunStart, 10),
            mark(MarkKind::EnterAlternate, 14),
        ]);
        assert_eq!(mode_of(&log), PRIVATE_LOG_MODE);

        fs::write(
            marks_path(&log),
            "run-start 7\nnot a mark\nrun-start x\nrun-start 9",
        )
        .unwrap();
        assert_eq!(stream_marks(&log).marks(), [mark(MarkKind::RunStart, 7)]);

        write_marks(&log, &StreamMarks::new(vec![mark(MarkKind::RunStart, 1)])).unwrap();
        assert_eq!(stream_marks(&log).marks(), [mark(MarkKind::RunStart, 1)]);
        assert_eq!(mode_of(&log), PRIVATE_LOG_MODE);
    }
}
