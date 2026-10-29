//! What the attach tests and their fixtures share: the test terminal's
//! size, the process attached to and the detach key.

use std::time::Duration;

use tempfile::TempDir;

use crate::{
    adapter::test_scratch::short_scratch,
    domain::{
        pty::PtySize,
        value::{DetachKey, ProcessName},
    },
};

/// Prefix of every test scratch directory.
const SCRATCH_PREFIX: &str = "ma-";
/// Rows of the test terminal.
pub(super) const ROWS: u16 = 24;
/// Columns of the test terminal.
pub(super) const COLS: u16 = 80;
/// Scrollback of the test holder.
pub(super) const SCROLLBACK: usize = 100;
/// First-supervisor deadline of a test holder, past any test's length.
pub(super) const SUPERVISOR_DEADLINE: Duration = Duration::from_secs(60);
/// The detach key every test uses.
const DETACH: &str = "ctrl-\\";
/// Pid a fake holder reports.
pub(super) const FAKE_PID: u32 = 4242;

/// The process every test attaches to.
pub(super) fn name() -> ProcessName {
    ProcessName::try_new("api").unwrap()
}

/// The detach key every test uses.
pub(super) fn detach() -> DetachKey {
    DetachKey::try_new(DETACH).unwrap()
}

/// A terminal size of `rows` by `cols`.
pub(super) fn size(rows: u16, cols: u16) -> PtySize {
    PtySize::try_new(rows, cols).unwrap()
}

/// A scratch directory under a short root, removed even on a panic.
pub(super) fn scratch() -> TempDir {
    short_scratch(SCRATCH_PREFIX)
}
