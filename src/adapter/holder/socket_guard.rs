use std::{
    os::unix::net::UnixStream,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use crate::adapter::ipc;

/// Stops the accept loop and removes the socket when the holder ends.
pub(super) struct SocketGuard {
    pub(super) path: PathBuf,
    pub(super) closing: Arc<AtomicBool>,
}

impl Drop for SocketGuard {
    /// Flags the acceptor, wakes it with a throwaway connection and unlinks the
    /// socket so no client can reach a holder that has stopped serving.
    fn drop(&mut self) {
        self.closing.store(true, Ordering::SeqCst);
        let _ = UnixStream::connect(&self.path);
        ipc::unlink(&self.path);
    }
}
