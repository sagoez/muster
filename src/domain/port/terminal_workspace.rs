use crate::domain::terminal_workspace::{OpenedPane, PaneRequest, WorkspaceError, WorkspaceProbe};

/// The terminal workspace the caller runs in, which can open a pane beside
/// the caller's.
pub trait TerminalWorkspace: Send + Sync {
    /// Opens a pane beside the caller's, running `request`'s command in its
    /// folder.
    ///
    /// # Errors
    /// Returns a [`WorkspaceError`] when the workspace cannot be reached,
    /// refuses, or answers unreadably.
    fn open_beside(&self, request: &PaneRequest) -> Result<OpenedPane, WorkspaceError>;

    /// Checks that the workspace answers for the caller's pane.
    ///
    /// # Errors
    /// As [`Self::open_beside`].
    fn check(&self) -> Result<WorkspaceProbe, WorkspaceError>;
}
