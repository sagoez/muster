use serde::{Deserialize, Serialize};

/// The reply the IPC tests answer a [`Ping`](super::ping::Ping) with.
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Pong {
    /// The one reply, carrying a number to tell replies apart.
    Pong(u32),
}
