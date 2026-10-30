use std::{
    thread,
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, RecvTimeoutError, TryRecvError};

/// Poll interval used while a direct child has exited but its process group is
/// still completing descendant cleanup.
const PROCESS_GROUP_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Outcome of waiting for both PTY output and descendant cleanup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum DrainStatus {
    Complete,
    TimedOut,
}

/// Waits until the PTY reader finishes and the process group is empty, bounded
/// by `grace`. The liveness callback keeps the timing logic directly testable.
pub(super) fn wait_for_drain<F>(
    reader_done_rx: &Receiver<()>,
    grace: Duration,
    mut group_is_alive: F,
) -> DrainStatus
where
    F: FnMut() -> bool,
{
    let started = Instant::now();
    let mut reader_done = match reader_done_rx.try_recv() {
        Ok(()) | Err(TryRecvError::Disconnected) => true,
        Err(TryRecvError::Empty) => false,
    };
    loop {
        if reader_done && !group_is_alive() {
            return DrainStatus::Complete;
        }

        let remaining = grace.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return DrainStatus::TimedOut;
        }
        let poll = remaining.min(PROCESS_GROUP_POLL_INTERVAL);
        if reader_done {
            thread::sleep(poll);
            continue;
        }
        match reader_done_rx.recv_timeout(poll) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => reader_done = true,
            Err(RecvTimeoutError::Timeout) => {},
        }
    }
}
