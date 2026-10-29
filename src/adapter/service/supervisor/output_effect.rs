use super::session_row::SessionRow;

/// What one output event did, for a caller that keeps state beside the
/// supervisor.
pub enum OutputEffect {
    /// The event was stale or for an unknown pane, and changed nothing.
    Ignored,
    /// The event reached the pane: a chunk updated its terminal and attention,
    /// or an unreadable stream was reported.
    Applied,
    /// The process exited; `retired` says its row left the workspace.
    Exited {
        /// Whether the exit removed the process from the workspace.
        retired: bool,
        /// Where a session reopen queued behind this exit landed.
        reopened: Option<SessionRow>,
    },
}
