use strum::Display;

/// What a running process appears to be doing, inferred from its terminal
/// signals. Orthogonal to [`ProcessState`](super::ProcessState): the lifecycle
/// says whether a child is alive, this says whether it is busy or waiting on the
/// user. The TUI maps it to a sidebar accent; the domain stays free of rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Display)]
#[strum(serialize_all = "lowercase")]
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
}
