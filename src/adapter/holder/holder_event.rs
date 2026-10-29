use std::os::unix::net::UnixStream;

use super::{client_id::ClientId, client_message::ClientMessage};
use crate::domain::pty::ProcessOutput;

/// One message for the holder loop.
pub(super) enum HolderEvent {
    /// The child wrote output or exited.
    Child(ProcessOutput),
    /// A client connected.
    Connected {
        /// The new connection's identity.
        client: ClientId,
        /// The connection's write half.
        stream: UnixStream,
    },
    /// A client sent a message.
    Message {
        /// The sender.
        client: ClientId,
        /// What it sent.
        message: ClientMessage,
    },
    /// A client's connection closed or sent something unreadable.
    Disconnected {
        /// The client that went away.
        client: ClientId,
    },
    /// A graceful stop's grace ran out; a child still running is killed.
    GraceElapsed,
    /// The first-supervisor deadline passed; a holder never supervised stops.
    SupervisorDeadline,
}
