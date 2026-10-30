//! The bounds every test that polls shares, and the poll itself.

use std::{
    thread,
    time::{Duration, Instant},
};

/// Bound on every wait; reaching it fails the test.
pub const WAIT: Duration = Duration::from_secs(10);
/// Interval between checks of a polled condition.
pub const POLL: Duration = Duration::from_millis(10);
/// Bound on one answer a test asks a socket for, past any wait it may hold.
pub const REPLY: Duration = Duration::from_secs(60);

/// Polls `condition` until it holds, failing the test at [`WAIT`].
pub fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let started = Instant::now();
    while !condition() {
        assert!(started.elapsed() < WAIT, "timed out waiting for {what}");
        thread::sleep(POLL);
    }
}
