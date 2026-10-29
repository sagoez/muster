use serde::{Deserialize, Serialize};
use strum::{Display, EnumIter, EnumString};

use crate::domain::pty::ExitOutcome;

/// How a process should be restarted when its child exits.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Display, EnumString, EnumIter,
)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum RestartPolicy {
    /// Never restart; a stopped process stays stopped.
    Never,
    /// Restart only when the process exits with a failure status or signal.
    OnFailure,
    /// Always restart, whatever the exit status.
    Always,
}

impl RestartPolicy {
    /// Whether a process with this policy should be restarted after `outcome`.
    pub fn should_restart(self, outcome: &ExitOutcome) -> bool {
        match self {
            Self::Never => false,
            Self::OnFailure => outcome.is_failure(),
            Self::Always => true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::pty::{ExitCause, SignalName};

    #[test]
    fn never_never_restarts() {
        assert!(!RestartPolicy::Never.should_restart(&ExitOutcome::FAILED_UNKNOWN));
        assert!(!RestartPolicy::Never.should_restart(&ExitOutcome::Succeeded));
    }

    #[test]
    fn on_failure_restarts_only_on_failure() {
        assert!(RestartPolicy::OnFailure.should_restart(&ExitOutcome::FAILED_UNKNOWN));
        assert!(
            RestartPolicy::OnFailure.should_restart(&ExitOutcome::Failed(ExitCause::Signal(
                SignalName::try_new("sigkill").unwrap()
            )))
        );
        assert!(!RestartPolicy::OnFailure.should_restart(&ExitOutcome::Succeeded));
    }

    #[test]
    fn always_restarts_regardless() {
        assert!(RestartPolicy::Always.should_restart(&ExitOutcome::Succeeded));
        assert!(RestartPolicy::Always.should_restart(&ExitOutcome::FAILED_UNKNOWN));
    }

    #[test]
    fn reads_its_kebab_case_name() {
        assert_eq!(
            "on-failure".parse::<RestartPolicy>().unwrap(),
            RestartPolicy::OnFailure
        );
        assert!("sometimes".parse::<RestartPolicy>().is_err());
    }
}
