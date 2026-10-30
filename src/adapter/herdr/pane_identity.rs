use crate::domain::coordination::HERDR_PANE_ENV;

/// The variables herdr sets to name the one pane, tab and workspace a shell
/// runs in. A process muster starts outlives that pane and must not claim
/// it, so no launch carries them.
pub const PANE_IDENTITY_ENV: [&str; 4] = [
    HERDR_PANE_ENV,
    "HERDR_PANE_RUNTIME_ID",
    "HERDR_TAB_ID",
    "HERDR_WORKSPACE_ID",
];
