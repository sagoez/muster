use super::session_row::SessionRow;

/// What reopening a closed agent session did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReopenOutcome {
    /// The session is running again on this row.
    Opened(SessionRow),
    /// The session's previous row is still shutting down; it reopens once that
    /// row has exited.
    Queued,
}
