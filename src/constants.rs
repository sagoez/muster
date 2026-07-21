/// Human-facing application name, reused for window titles and labels.
pub const APP_NAME: &str = "muster";

/// Environment variable exported into each spawned pane, holding the path of the
/// current project's config so the `muster` CLI can target it without a flag.
pub const MUSTER_PROJECT_ENV: &str = "MUSTER_PROJECT";
