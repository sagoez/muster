use std::time::Duration;

use muster_domain::process::StopSignal;
use serde::{Deserialize, Serialize};

/// A process control request, honoured only from a supervisor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", rename_all_fields = "kebab-case")]
pub enum HolderControl {
    /// Suspends the child's process group.
    Pause,
    /// Resumes a suspended child's process group.
    Resume,
    /// Sends `signal`, allowing `grace` before the group is force-killed.
    Terminate {
        /// The graceful signal to send.
        #[serde(with = "crate::adapter::wire::form")]
        signal: StopSignal,
        /// Time allowed for a graceful exit.
        grace: Duration,
    },
    /// Force-kills the child's process group.
    Kill,
}
