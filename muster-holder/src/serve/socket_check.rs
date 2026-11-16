use muster_protocol::holder::HolderError;

/// What one look at the holder's socket path found, and did.
pub(super) enum SocketCheck {
    /// The socket the holder bound is still there.
    Intact,
    /// The socket was gone, so the holder bound it again and serves it from a
    /// fresh acceptor, the old one stopped.
    Rebound,
    /// Something else now sits at the path, which the holder never takes.
    Occupied,
    /// The socket was gone, and binding it again failed: a folder on its path
    /// is not private, or creating or binding it failed.
    Refused(HolderError),
}
