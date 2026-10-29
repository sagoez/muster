//! What the holder server's tests and their fixtures share: bounds, the test
//! child's request and a scratch directory.

use std::{
    path::Path,
    thread,
    time::{Duration, Instant},
};

use tempfile::TempDir;

use super::testing::test_request;
use crate::{
    adapter::test_scratch::short_scratch,
    domain::{
        color_scheme::ColorScheme,
        pty::{PtySize, SpawnRequest},
    },
};

/// Bound on every wait in these tests; reaching it fails the test.
pub(super) const WAIT: Duration = Duration::from_secs(10);
/// First-supervisor deadline of a test holder: past any test's length, so
/// only the deadline test ever reaches one.
pub(super) const SUPERVISOR_DEADLINE: Duration = Duration::from_secs(60);
/// Interval between checks of a polled condition.
pub(super) const POLL: Duration = Duration::from_millis(10);
/// Prefix of every test scratch directory.
const SCRATCH_PREFIX: &str = "mh-";
/// Initial PTY rows of every test child.
pub(super) const ROWS: u16 = 24;
/// Initial PTY cols of every test child.
pub(super) const COLS: u16 = 80;
/// Scrollback the test holder keeps.
pub(super) const SCROLLBACK: usize = 100;
/// Directory the holder creates for its socket inside the scratch dir.
pub(super) const SOCKET_DIR: &str = "sock";
/// File name of the test holder's socket.
pub(super) const SOCKET_FILE: &str = "h.sock";
/// Pid the fake handle reports.
pub(super) const FAKE_PID: u32 = 4242;
/// Why the fake handle refuses a resize.
pub(super) const RESIZE_REFUSED: &str = "resize refused";

/// A PTY size of `rows` by `cols`.
pub(super) fn size(rows: u16, cols: u16) -> PtySize {
    PtySize::try_new(rows, cols).unwrap()
}

/// A request to run `command` as `dev server` of the project at `dir`,
/// whose terminal answers color queries from `color_scheme`.
pub(super) fn request(dir: &Path, command: &str, color_scheme: ColorScheme) -> SpawnRequest {
    let request = test_request("dev server", command, Some(dir.to_path_buf()), None);
    SpawnRequest::builder()
        .process(request.process().clone())
        .command(request.command().clone())
        .project(request.project().clone())
        .size(size(ROWS, COLS))
        .scrollback(SCROLLBACK)
        .log_max_size(request.log_max_size())
        .color_scheme(color_scheme)
        .stop(request.stop())
        .build()
}

/// Polls `condition` until it holds, failing the test at [`WAIT`].
pub(super) fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let started = Instant::now();
    while !condition() {
        assert!(started.elapsed() < WAIT, "timed out waiting for {what}");
        thread::sleep(POLL);
    }
}

/// A scratch directory under a short root, removed even when a test panics.
pub(super) fn scratch() -> TempDir {
    short_scratch(SCRATCH_PREFIX)
}
