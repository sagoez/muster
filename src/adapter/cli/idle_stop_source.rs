use serde::Serialize;

/// Where a project's effective idle stop comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum IdleStopSource {
    /// The project's own override.
    Project,
    /// The `idle-stop` setting, which the project follows.
    Setting,
}
