use std::time::Duration;

use serde::{Deserialize, Serialize};
use strum::{Display, EnumIter, EnumString};

use crate::domain::process::ProcessState;

/// How long a run must have lasted before it counts as running: a launch
/// report and a wait for `running` hold on this long, so a process that exits
/// at once is reported crashed or exited rather than started.
pub const QUICK_EXIT_WINDOW: Duration = Duration::from_millis(500);

/// A lifecycle state a wait can be for: one `ps` shows, or `stopped` for not
/// running for any reason.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Display, EnumString, EnumIter, Serialize, Deserialize,
)]
#[strum(serialize_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
pub enum StateGoal {
    /// Running.
    Running,
    /// Paused.
    Paused,
    /// Exited with status 0.
    Exited,
    /// Crashed.
    Crashed,
    /// Waiting out a restart backoff.
    Restarting,
    /// Not running, whatever the reason.
    Stopped,
}

impl StateGoal {
    /// Whether a process in `state` has reached this goal.
    pub fn reached(self, state: ProcessState) -> bool {
        match self {
            Self::Running => state == ProcessState::Running,
            Self::Paused => state == ProcessState::Paused,
            Self::Exited => state == ProcessState::Exited,
            Self::Crashed => state == ProcessState::Crashed,
            Self::Restarting => state == ProcessState::Restarting,
            Self::Stopped => !state.may_be_running(),
        }
    }
}
