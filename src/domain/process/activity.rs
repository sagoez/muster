use serde::{Deserialize, Serialize};
use strum::Display;

/// What a running process appears to be doing, inferred from its terminal
/// signals. Orthogonal to [`ProcessState`](super::ProcessState): the lifecycle
/// says whether a child is alive, this says whether it is busy or waiting on the
/// user. `muster ps --json` reports it beside the state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Display, Serialize, Deserialize)]
#[strum(serialize_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
pub enum ActivityState {
    /// No recent signal: freshly started, stopped, or between bursts of work.
    #[default]
    Idle,
    /// Producing output or reporting progress right now.
    Working,
    /// Signalled that it wants the user, via a bell or a notification sequence
    /// (e.g. an agent that finished and is waiting for the next instruction).
    AwaitingInput,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_idle() {
        assert_eq!(ActivityState::default(), ActivityState::Idle);
    }

    /// Display and the wire name the same kebab-case token.
    #[test]
    fn displays_in_kebab_case_like_the_wire() {
        assert_eq!(ActivityState::AwaitingInput.to_string(), "awaiting-input");
        assert_eq!(
            serde_json::to_value(ActivityState::AwaitingInput).unwrap(),
            "awaiting-input"
        );
    }
}
