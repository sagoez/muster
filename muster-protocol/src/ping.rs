use serde::{Deserialize, Serialize};

/// The request the IPC tests send.
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Ping {
    /// The one request.
    Ping,
}
