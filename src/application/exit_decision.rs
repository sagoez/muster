use crate::domain::process::ProcessState;

/// Next lifecycle action after a managed child exits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitDecision {
    /// Remove a pane whose process was removed from the stack.
    Retire,
    /// Keep the pane stopped after an explicit user stop.
    Stop,
    /// Respawn immediately after a requested restart.
    RestartNow,
    /// Schedule a policy restart after backoff.
    RestartLater,
    /// Keep the pane stopped with the terminal exit disposition.
    Settle(ProcessState),
}
