use serde::{Deserialize, Serialize};

/// How serious a [`Notice`](super::Notice) is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NoticeLevel {
    /// Supervision recovered on its own, but the user should know.
    Warning,
    /// An action failed and the process is not in the state that was asked for.
    Error,
}
