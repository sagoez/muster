use serde::{Deserialize, Serialize};

/// A lifecycle action a client asks the service to apply to one process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProcessAction {
    /// Start it unless it is already running.
    Start,
    /// Stop it through its stop policy, without its restart policy respawning
    /// it.
    Stop,
    /// Restart it regardless of its restart policy.
    Restart,
    /// Suspend it, leaving it alive.
    Pause,
    /// Resume it after a pause.
    Resume,
    /// Kill it at once, skipping its graceful stop.
    Kill,
}
