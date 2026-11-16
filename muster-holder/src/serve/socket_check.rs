use std::os::unix::net::UnixListener;

use muster_protocol::holder::HolderError;

/// What one look at the holder's socket path found, and did.
pub(super) enum SocketCheck {
    /// The socket the holder bound is still there.
    Intact,
    /// The socket was gone, so the holder bound it again: the new listener,
    /// for a fresh acceptor.
    Rebound(UnixListener),
    /// Something else now sits at the path, which the holder never takes.
    Occupied,
    /// The socket was gone, and binding it again failed: a folder on its path
    /// is not private, or creating or binding it failed.
    Refused(HolderError),
}
