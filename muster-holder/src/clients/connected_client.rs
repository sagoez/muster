use std::os::unix::net::UnixStream;

use muster_protocol::holder::{ClientRole, Frame};

use crate::serve::byte_queue::ByteQueue;

/// A connected client as the holder loop tracks it.
pub(crate) struct Client {
    /// Its role, fixed by its hello; `None` until then.
    pub(crate) role: Option<ClientRole>,
    /// The connection, kept to shut it down when the client is dropped.
    pub(crate) stream: UnixStream,
    /// Frames awaiting its writer thread; dropping it lets the writer finish.
    pub(crate) outbox: ByteQueue<Frame>,
}
