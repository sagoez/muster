use std::os::unix::net::UnixStream;

use super::{byte_queue::ByteQueue, client_role::ClientRole, frame::Frame};

/// A connected client as the holder loop tracks it.
pub(super) struct Client {
    /// Its role, fixed by its hello; `None` until then.
    pub(super) role: Option<ClientRole>,
    /// The connection, kept to shut it down when the client is dropped.
    pub(super) stream: UnixStream,
    /// Frames awaiting its writer thread; dropping it lets the writer finish.
    pub(super) outbox: ByteQueue<Frame>,
}
