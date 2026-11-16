#[cfg(test)]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::clients::client_id::ClientId;

/// What every acceptor of one holder's socket shares: the next connection's
/// number, so a connection to a socket bound again never takes the number of
/// one still open.
#[derive(Default)]
pub(in crate::serve) struct AcceptorCounts {
    /// The next connection's number.
    connections: AtomicU64,
    /// Whether a test makes every acceptor start fail.
    #[cfg(test)]
    refusing: AtomicBool,
}

impl AcceptorCounts {
    /// The identity of the next connection accepted.
    pub(super) fn next_client(&self) -> ClientId {
        ClientId::new(self.connections.fetch_add(1, Ordering::SeqCst))
    }

    /// Makes every acceptor start fail while `refusing`, as one that cannot
    /// get a thread does.
    #[cfg(test)]
    pub(in crate::serve) fn refuse_starts(&self, refusing: bool) {
        self.refusing.store(refusing, Ordering::SeqCst);
    }

    /// Whether a test makes acceptor starts fail.
    #[cfg(test)]
    pub(super) fn refuses_starts(&self) -> bool {
        self.refusing.load(Ordering::SeqCst)
    }
}
