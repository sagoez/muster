use strum::Display;

use super::workspace_error::WorkspaceError;

/// What became of a pane muster could not run in.
#[derive(Debug, Clone, PartialEq, Eq, Display)]
#[strum(serialize_all = "kebab-case")]
pub enum PaneLeft {
    /// It was closed.
    Closed,
    /// It is still open, as closing it failed too, for the reason held.
    Open(Box<WorkspaceError>),
}
