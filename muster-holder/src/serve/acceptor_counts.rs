#[cfg(test)]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

use crate::clients::client_id::ClientId;

/// What every acceptor of one holder's socket shares: the next connection's
/// number, so a connection to a socket bound again never takes the number of
/// one still open, and how many acceptor threads run and listeners are
/// open, so a rebind is seen to leave exactly one of each.
#[derive(Default)]
pub(super) struct AcceptorCounts {
    /// The next connection's number.
    connections: AtomicU64,
    /// Acceptor threads that have not returned.
    running: AtomicUsize,
    /// Listeners still open.
    listening: AtomicUsize,
    /// Whether a test makes every acceptor start fail.
    #[cfg(test)]
    refusing: AtomicBool,
}

impl AcceptorCounts {
    /// The identity of the next connection accepted.
    pub(super) fn next_client(&self) -> ClientId {
        ClientId::new(self.connections.fetch_add(1, Ordering::SeqCst))
    }

    /// Counts an acceptor thread started.
    pub(super) fn thread_started(&self) {
        self.running.fetch_add(1, Ordering::SeqCst);
    }

    /// Counts an acceptor thread returned.
    pub(super) fn thread_returned(&self) {
        self.running.fetch_sub(1, Ordering::SeqCst);
    }

    /// Counts a listener opened.
    pub(super) fn listener_opened(&self) {
        self.listening.fetch_add(1, Ordering::SeqCst);
    }

    /// Counts a listener closed.
    pub(super) fn listener_closed(&self) {
        self.listening.fetch_sub(1, Ordering::SeqCst);
    }

    /// How many acceptor threads run.
    #[cfg(test)]
    pub(super) fn running(&self) -> usize {
        self.running.load(Ordering::SeqCst)
    }

    /// How many listeners are open.
    #[cfg(test)]
    pub(super) fn listening(&self) -> usize {
        self.listening.load(Ordering::SeqCst)
    }

    /// Makes every acceptor start fail while `refusing`, as one that cannot
    /// get a thread does.
    #[cfg(test)]
    pub(super) fn refuse_starts(&self, refusing: bool) {
        self.refusing.store(refusing, Ordering::SeqCst);
    }

    /// Whether a test makes acceptor starts fail.
    #[cfg(test)]
    pub(super) fn refuses_starts(&self) -> bool {
        self.refusing.load(Ordering::SeqCst)
    }
}
