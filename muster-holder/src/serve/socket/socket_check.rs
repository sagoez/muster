use std::path::Path;

use muster_protocol::holder::HolderError;
use strum::{EnumDiscriminants, EnumMessage};

/// What one look at the holder's socket path found, and did, with what the
/// supervisor is told of it. Its kinds, [`SocketTrouble`], are what the
/// holder warns of, so trouble that lasts is warned of once.
#[derive(EnumDiscriminants, EnumMessage)]
#[strum_discriminants(name(SocketTrouble))]
pub(in crate::serve) enum SocketCheck {
    /// The socket the holder bound is still there.
    Intact,
    /// The socket was gone, so the holder bound it again and serves it from
    /// a fresh acceptor, the old one let go.
    #[strum(message = "the holder's socket was removed; bound it again at")]
    Rebound,
    /// Something else now sits at the path, which the holder never takes.
    #[strum(
        message = "the holder's socket was replaced by a file it did not bind, which it leaves \
                   alone until it is removed; clients cannot reach it at"
    )]
    Occupied,
    /// The socket was gone, and binding it again failed: a folder on its
    /// path is not private, or creating, binding or serving it failed.
    #[strum(message = "the holder's socket was removed and could not be bound again at")]
    Refused(HolderError),
}

impl SocketCheck {
    /// What the supervisor is told of this look at the socket at `path`:
    /// its message, the path, and why a refused bind failed.
    pub(in crate::serve) fn warning(&self, path: &Path) -> String {
        let told = self.get_message().unwrap_or_default();
        match self {
            Self::Refused(error) => format!("{told} {}: {error}", path.display()),
            Self::Intact | Self::Rebound | Self::Occupied => {
                format!("{told} {}", path.display())
            },
        }
    }
}
