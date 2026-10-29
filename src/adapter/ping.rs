use serde::{Deserialize, Serialize};

/// The request the IPC tests send.
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
pub(crate) enum Ping {
    /// The one request.
    Ping,
}
