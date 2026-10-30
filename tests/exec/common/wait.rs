use std::{
    thread,
    time::{Duration, Instant},
};

/// Bound on every wait; reaching it fails the test.
pub const WAIT: Duration = Duration::from_secs(30);
/// Interval between checks of a polled condition.
pub const POLL: Duration = Duration::from_millis(10);

/// Polls `condition` until it holds, failing the test at [`WAIT`].
pub fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let started = Instant::now();
    while !condition() {
        assert!(started.elapsed() < WAIT, "timed out waiting for {what}");
        thread::sleep(POLL);
    }
}
