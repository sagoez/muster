use serde::{Deserialize, Serialize};
use strum::Display;

use crate::domain::pty::ExitOutcome;

/// How a process should be restarted when its child exits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum RestartPolicy {
    /// Never restart; a stopped process stays stopped.
    #[default]
    Never,
    /// Restart only when the process exits with a failure status or signal.
    OnFailure,
    /// Always restart, whatever the exit status.
    Always,
}

impl RestartPolicy {
    /// Whether a process with this policy should be restarted after `outcome`.
    pub fn should_restart(self, outcome: ExitOutcome) -> bool {
        match self {
            Self::Never => false,
            Self::OnFailure => outcome == ExitOutcome::Failed,
            Self::Always => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn never_never_restarts() {
        assert!(!RestartPolicy::Never.should_restart(ExitOutcome::Failed));
        assert!(!RestartPolicy::Never.should_restart(ExitOutcome::Succeeded));
    }

    #[test]
    fn on_failure_restarts_only_on_failure() {
        assert!(RestartPolicy::OnFailure.should_restart(ExitOutcome::Failed));
        assert!(!RestartPolicy::OnFailure.should_restart(ExitOutcome::Succeeded));
    }

    #[test]
    fn always_restarts_regardless() {
        assert!(RestartPolicy::Always.should_restart(ExitOutcome::Succeeded));
        assert!(RestartPolicy::Always.should_restart(ExitOutcome::Failed));
    }

    #[test]
    fn defaults_to_never() {
        assert_eq!(RestartPolicy::default(), RestartPolicy::Never);
    }
}
