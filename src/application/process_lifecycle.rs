use super::exit_decision::ExitDecision;
use crate::domain::{
    process::{ExitIntent, ProcessState},
    pty::ExitOutcome,
};

/// Pure process lifecycle policy used by the supervisor.
pub struct ProcessLifecycle;

impl ProcessLifecycle {
    /// Chooses the post-exit action from explicit intent and restart policy.
    pub fn after_exit(
        intent: ExitIntent,
        retired_from_stack: bool,
        should_restart: bool,
        outcome: &ExitOutcome,
    ) -> ExitDecision {
        if retired_from_stack {
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
fn exit_state(outcome: &ExitOutcome) -> ProcessState {
    match outcome {
        ExitOutcome::Succeeded => ProcessState::Exited,
        ExitOutcome::Failed(_) => ProcessState::Crashed,
    }
}
