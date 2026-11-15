use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use crossbeam_channel::{Receiver, RecvTimeoutError};

/// How a poller waits between reads: for its period, or less when the hub
/// wakes it, and not at all once the hub is gone. A poller that reads only
/// for the pages also parks while no page streams, until the hub wakes it.
#[derive(Clone, Debug)]
pub struct Pause {
    /// What the hub wakes it through; disconnected once the hub ends.
    signal: Receiver<()>,
    /// Whether any page streams, for a poller that parks while none does.
    watched: Option<Arc<AtomicBool>>,
}

impl Pause {
    /// A pause woken through `signal`, that never parks.
    pub(super) fn new(signal: Receiver<()>) -> Self {
        Self {
            signal,
            watched: None,
        }
    }

    /// A pause woken through `signal`, that parks while `watched` is false.
    pub(super) fn parking(signal: Receiver<()>, watched: Arc<AtomicBool>) -> Self {
        Self {
            signal,
            watched: Some(watched),
        }
    }

    /// Waits `period`, or until woken, then, for a parking pause, for as
    /// long as no page streams; false once the hub is gone, so the poller
    /// ends.
    pub fn wait(&self, period: Duration) -> bool {
        if matches!(
            self.signal.recv_timeout(period),
            Err(RecvTimeoutError::Disconnected)
        ) {
            return false;
        }
        let Some(watched) = &self.watched else {
            return true;
        };
        while !watched.load(Ordering::SeqCst) {
            if self.signal.recv().is_err() {
                return false;
            }
        }
        true
    }
}
