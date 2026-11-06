use super::carried_pane::CarriedPane;

/// Why a pane is starting, which decides how its result is settled.
#[derive(Debug, Clone, Copy)]
pub(crate) enum LaunchPurpose {
    /// A new process is being launched.
    Spawn {
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
