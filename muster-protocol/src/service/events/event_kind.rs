use serde::{Deserialize, Serialize};
use strum::{Display, EnumIter, EnumString};

/// What happened to a process, as the event log records it.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Display, EnumString, EnumIter, Serialize, Deserialize,
)]
#[strum(serialize_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
pub enum EventKind {
    /// A run started.
    Started,
    /// A run passed its readiness check.
    Ready,
    /// A run did not pass its readiness check in time.
    ReadyTimeout,
    /// A run ended with status 0, or failed while being stopped.
    Exited,
    /// A run failed on its own.
    Crashed,
    /// A run was stopped so a restart could start it again. A run that ends
    /// on its own is `exited` or `crashed`, whether or not its policy
    /// restarts it.
    Restarting,
    /// A run was stopped on request.
    Stopped,
    /// A start was refused because something else holds what it needs.
    Blocked,
    /// A warning or error notice was raised.
    Notice,
    /// The project was stopped because no shell was in it for its idle stop.
    IdleStopped,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_stopped_is_kebab_case() {
        assert_eq!(EventKind::IdleStopped.to_string(), "idle-stopped");
        assert_eq!(
            serde_json::to_string(&EventKind::IdleStopped).unwrap(),
            r#""idle-stopped""#
        );
        assert_eq!("idle-stopped".parse(), Ok(EventKind::IdleStopped));
    }
}
