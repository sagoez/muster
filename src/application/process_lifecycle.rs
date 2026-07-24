use crate::domain::{
    process::{ExitIntent, ProcessState},
    pty::ExitOutcome,
};

/// Next lifecycle action after a managed child exits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitDecision {
    /// Remove a pane retired by configuration reconciliation.
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

/// Pure process lifecycle policy used by the TUI runtime.
pub struct ProcessLifecycle;

impl ProcessLifecycle {
    /// Chooses the post-exit action from explicit intent and restart policy.
    pub fn after_exit(
        intent: ExitIntent,
        retired_by_config: bool,
        should_restart: bool,
        outcome: ExitOutcome,
    ) -> ExitDecision {
        if retired_by_config {
            return ExitDecision::Retire;
        }
        match intent {
            ExitIntent::StopRetryable | ExitIntent::StopInFlight => ExitDecision::Stop,
            ExitIntent::RestartRetryable | ExitIntent::RestartInFlight => ExitDecision::RestartNow,
            ExitIntent::FollowPolicy if should_restart => ExitDecision::RestartLater,
            ExitIntent::FollowPolicy => ExitDecision::Settle(exit_state(outcome)),
        }
    }
}

/// Maps an OS process outcome to the state rendered for an inactive pane.
fn exit_state(outcome: ExitOutcome) -> ProcessState {
    match outcome {
        ExitOutcome::Succeeded => ProcessState::Exited,
        ExitOutcome::Failed => ProcessState::Crashed,
    }
}
