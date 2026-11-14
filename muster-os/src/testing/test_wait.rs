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
pub fn wait_until(what: &str, condition: impl FnMut() -> bool) {
    wait_within(what, WAIT, condition);
}

/// Polls `condition` until it holds, failing the test at `bound`, for a
/// wait that first sits out a fixed interval of the code under test.
pub fn wait_within(what: &str, bound: Duration, mut condition: impl FnMut() -> bool) {
    let started = Instant::now();
    while !condition() {
        assert!(started.elapsed() < bound, "timed out waiting for {what}");
        thread::sleep(POLL);
    }
}

#[cfg(test)]
mod tests {
    use std::panic;

    use super::*;

    /// A bound far shorter than any test's wait, to see one run out.
    const SHORT_BOUND: Duration = Duration::from_millis(50);

    /// A wait returns once its condition holds within its own bound, which
    /// may outlast [`WAIT`], and fails the test once that bound runs out.
    #[test]
    fn a_wait_within_holds_until_its_own_bound() {
        let started = Instant::now();
        wait_within("a delay", WAIT.saturating_mul(2), || {
            started.elapsed() >= POLL.saturating_mul(3)
        });

        let started = Instant::now();
        let timed_out = panic::catch_unwind(|| wait_within("nothing", SHORT_BOUND, || false));
        assert!(timed_out.is_err());
        assert!(started.elapsed() >= SHORT_BOUND);
    }
}
