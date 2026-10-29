/// Why a pane is being launched, which selects the resume semantics for a linked
/// agent session. Automatic launches (autostart, a policy-driven restart) use
/// `AgentSession::restore_command`, so an owned-but-uncaptured reporter session
/// is surfaced as stopped rather than silently relaunched as a fresh
/// conversation; an explicit user launch uses `AgentSession::reopen_command`,
/// starting a fresh conversation when there is nothing to resume.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchIntent {
    /// Muster initiated the launch (autostart or an automatic restart).
    Automatic,
    /// The user explicitly asked for the launch (start or restart action).
    UserInitiated,
}
