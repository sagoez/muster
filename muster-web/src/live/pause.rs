use std::time::Duration;

use crossbeam_channel::{Receiver, RecvTimeoutError};

/// How a poller waits between reads: for its period, or less when the hub
/// wakes it, and not at all once the hub is gone.
#[derive(Clone, Debug)]
pub struct Pause {
    /// What the hub wakes it through; disconnected once the hub ends.
    signal: Receiver<()>,
}

impl Pause {
    /// A pause woken through `signal`.
    pub(super) fn new(signal: Receiver<()>) -> Self {
        Self { signal }
    }

    /// Waits `period`, or until woken; false once the hub is gone, so the
    /// poller ends.
    pub fn wait(&self, period: Duration) -> bool {
        !matches!(
            self.signal.recv_timeout(period),
            Err(RecvTimeoutError::Disconnected)
        )
    }
}
