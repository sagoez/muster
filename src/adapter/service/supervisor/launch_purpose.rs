use super::carried_pane::CarriedPane;
use crate::domain::agent_session::{AgentSessionId, LaunchToken};

/// Why a pane is starting, which decides how its result is settled.
#[derive(Debug)]
pub(crate) enum LaunchPurpose {
    /// A new process is being launched.
    Spawn {
        /// The agent session it runs, which the launch claims once it runs.
        agent_session_id: Option<AgentSessionId>,
        /// The launch's token, recorded with that claim.
        launch_token: Option<LaunchToken>,
        /// Whether the launch is a restart, which `ps` counts.
        restarting: bool,
    },
    /// A running holder is being adopted; its record comes back with the
    /// result.
    Adopt {
        /// What the pane carries over when this is a reconnect.
        carried: Option<CarriedPane>,
    },
}
