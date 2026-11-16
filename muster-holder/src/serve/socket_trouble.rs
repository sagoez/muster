/// What kind of trouble the holder last warned of about its socket, so
/// trouble that lasts is warned of once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SocketTrouble {
    /// The socket was removed, or stopped accepting, and bound again.
    Rebound,
    /// Something else sits at the socket path.
    Occupied,
    /// The socket was removed and could not be bound again.
    Refused,
}
