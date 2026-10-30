//! Where each run a log holds began, kept in a `.runs` sidecar beside the
//! log, so every reader renders each run from a fresh terminal: a run killed
//! on the alternate screen, mid-color or mid-escape never reaches the next.

use std::{
    ffi::OsString,
    fs,
    io::{self, Write},
    os::unix::fs::OpenOptionsExt as _,
    path::{Path, PathBuf},
};

use atomic_write_file::{OpenOptions, unix::OpenOptionsExt as _};

use super::log::PRIVATE_LOG_MODE;
use crate::domain::value::OutputCursor;

/// Suffix of the sidecar that records where each run began.
const RUNS_SUFFIX: &str = ".runs";
/// Most run starts the sidecar keeps, the newest; one far older than any a
/// log of the largest size still holds is of no use to a reader.
const MAX_RUN_STARTS: usize = 256;

/// The path of the sidecar beside the log at `log`.
fn runs_path(log: &Path) -> PathBuf {
    let mut runs = OsString::from(log.as_os_str());
    runs.push(RUNS_SUFFIX);
    PathBuf::from(runs)
}

/// Where each run of the log at `log` began, oldest first; none when the
/// sidecar is absent, and a line that does not parse is skipped.
pub fn run_starts(log: &Path) -> Vec<OutputCursor> {
    let Ok(text) = fs::read_to_string(runs_path(log)) else {
        return Vec::new();
    };
    let mut starts: Vec<OutputCursor> = text
        .lines()
        .filter_map(|line| line.trim().parse().ok())
        .map(OutputCursor::new)
        .collect();
    starts.sort_unstable();
    starts.dedup();
    starts
}

/// Records that a run of the log at `log` begins at `at`, before it writes
/// anything, so a reader that reads the run's output and then the sidecar
/// always finds its start. Only the newest [`MAX_RUN_STARTS`] are kept, and
/// the sidecar is replaced atomically, its owner's alone like the log.
///
/// # Errors
/// Returns an [`io::Error`] when the sidecar cannot be written.
pub(super) fn record_run_start(log: &Path, at: OutputCursor) -> io::Result<()> {
    let mut starts = run_starts(log);
    starts.retain(|start| *start < at);
    starts.push(at);
    let kept = &starts[starts.len().saturating_sub(MAX_RUN_STARTS)..];
    let mut file = OpenOptions::new()
        .preserve_mode(false)
        .mode(PRIVATE_LOG_MODE)
        .open(runs_path(log))?;
    for start in kept {
        writeln!(file, "{}", start.offset())?;
    }
    file.commit()
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use tempfile::TempDir;

    use super::*;

    /// Mode bits a permission check compares.
    const PERMISSION_BITS: u32 = 0o777;

    /// Run starts read back in order, private, bounded to the newest, and a
    /// start recorded again after a reopen at an older end replaces the
    /// newer ones it can no longer follow.
    #[test]
    fn run_starts_round_trip_bounded_and_private() {
        let scratch = TempDir::new().unwrap();
        let log = scratch.path().join("api.log");
        assert!(run_starts(&log).is_empty());

        for offset in 0..MAX_RUN_STARTS as u64 + 10 {
            record_run_start(&log, OutputCursor::new(offset * 10)).unwrap();
        }

        let starts = run_starts(&log);
        assert_eq!(starts.len(), MAX_RUN_STARTS);
        assert_eq!(
            starts.last().copied(),
            Some(OutputCursor::new((MAX_RUN_STARTS as u64 + 9) * 10))
        );
        let mode = fs::metadata(runs_path(&log)).unwrap().permissions().mode() & PERMISSION_BITS;
        assert_eq!(mode, PRIVATE_LOG_MODE);

        record_run_start(&log, OutputCursor::new(15)).unwrap();
        assert_eq!(run_starts(&log), [OutputCursor::new(15)]);

        fs::write(runs_path(&log), "7\nnot a start\n3\n").unwrap();
        assert_eq!(run_starts(&log), [
            OutputCursor::new(3),
            OutputCursor::new(7)
        ]);
    }
}
