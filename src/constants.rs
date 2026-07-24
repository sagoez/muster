/// Human-facing application name, reused for window titles and labels.
pub const APP_NAME: &str = "muster";

/// Environment variable exported into each spawned pane, holding the path of the
/// current project's config so the `muster` CLI can target it without a flag.
pub const MUSTER_PROJECT_ENV: &str = "MUSTER_PROJECT";
/// Internal agent-session identity inherited by provider lifecycle hooks.
pub const MUSTER_AGENT_SESSION_ENV: &str = "MUSTER_AGENT_SESSION_ID";
/// Exact durable session-state file inherited by provider lifecycle hooks.
pub const MUSTER_AGENT_SESSION_STATE_FILE_ENV: &str = "MUSTER_AGENT_SESSION_STATE_FILE";
