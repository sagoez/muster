use serde::{Deserialize, Serialize};

/// How a connected client participates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", rename_all_fields = "kebab-case")]
pub enum ClientRole {
    /// The service: writes, resizes, controls the child and collects its exit.
    Supervisor,
    /// A user terminal; a read-only one only watches.
    Attach {
        /// Whether input and resizes from this client are refused.
        read_only: bool,
    },
}

impl ClientRole {
    /// Whether this client may write input and resize the PTY.
    pub fn writable(self) -> bool {
        !matches!(self, Self::Attach { read_only: true })
    }
}
