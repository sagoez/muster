use std::{os::unix::net::UnixListener, sync::Arc};

use crate::serve::acceptor_counts::AcceptorCounts;

/// A listener of the holder's socket, counted open in [`AcceptorCounts`]
/// until it closes. An acceptor and its thread share it, so it stays open,
/// and keeps the socket's file from being freed, until both let go.
pub(super) struct OpenListener {
    /// The listening socket.
    listener: UnixListener,
    /// Where it is counted.
    counts: Arc<AcceptorCounts>,
}

impl OpenListener {
    /// Counts `listener` open in `counts`.
    pub(super) fn new(listener: UnixListener, counts: Arc<AcceptorCounts>) -> Self {
        counts.listener_opened();
        Self { listener, counts }
    }

    /// The listening socket.
    pub(super) fn listener(&self) -> &UnixListener {
        &self.listener
    }
}

impl Drop for OpenListener {
    /// Counts the listener closed as it closes.
    fn drop(&mut self) {
        self.counts.listener_closed();
    }
}
