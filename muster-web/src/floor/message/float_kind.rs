use serde::Serialize;

/// What rises off a process's face for a moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum FloatKind {
    /// It began listening on a port.
    Port,
    /// It was restarted once more.
    Restart,
}
