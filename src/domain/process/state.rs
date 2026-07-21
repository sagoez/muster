use strum::Display;

/// Lifecycle state of a managed process. The TUI adapter maps this to a sidebar
/// status glyph and color; the domain itself stays free of rendering concerns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Display)]
#[strum(serialize_all = "lowercase")]
pub enum ProcessState {
    /// Configured but not yet started.
    #[default]
    Pending,
    /// Child process is alive.
    Running,
    /// Child process is alive but suspended (SIGSTOP).
    Paused,
    /// Exited successfully (status 0).
    Exited,
    /// Exited with a failure status or was killed by a signal.
    Crashed,
    /// Scheduled to restart after a backoff.
    Restarting,
}

impl ProcessState {
    /// Whether the process currently has a live child attached.
    pub fn is_active(self) -> bool {
        matches!(self, Self::Running | Self::Paused | Self::Restarting)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_pending() {
        assert_eq!(ProcessState::default(), ProcessState::Pending);
    }

    #[test]
    fn active_only_while_running_or_restarting() {
        assert!(ProcessState::Running.is_active());
        assert!(ProcessState::Paused.is_active());
        assert!(ProcessState::Restarting.is_active());
        assert!(!ProcessState::Pending.is_active());
        assert!(!ProcessState::Exited.is_active());
        assert!(!ProcessState::Crashed.is_active());
    }
}
