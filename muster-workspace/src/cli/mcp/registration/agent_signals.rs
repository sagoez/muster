use std::sync::{Arc, atomic::AtomicBool};

use muster_os::signal_hold::SignalHold;
use signal_hook::{
    SigId,
    consts::{SIGINT, SIGQUIT},
    flag::register,
    low_level::unregister,
};

/// Keeps `SIGINT` and `SIGQUIT` from ending muster while an agent's CLI
/// runs in the foreground: the terminal sends them to the agent too, which
/// decides, and muster waits to report how it ended. Both are caught by a
/// handler, never ignored, so the agent starts with their default action.
/// An RAII guard, so it has a hand-written constructor.
pub struct AgentSignals {
    /// The hold on `SIGINT`, unless it could not be taken.
    _interrupt: Option<SignalHold>,
    /// The handler catching `SIGQUIT`, unless it could not be registered.
    quit: Option<SigId>,
}

impl AgentSignals {
    /// Catches both signals until the guard drops; one that cannot be
    /// caught keeps its usual action.
    pub fn hold() -> Self {
        Self {
            _interrupt: SignalHold::take(SIGINT).ok(),
            quit: register(SIGQUIT, Arc::new(AtomicBool::new(false))).ok(),
        }
    }
}

impl Drop for AgentSignals {
    /// Drops the `SIGQUIT` action, and the hold lets `SIGINT` end muster
    /// again. signal-hook keeps its handler for `SIGQUIT`, which then does
    /// nothing, for the little muster runs after the agent.
    fn drop(&mut self) {
        if let Some(quit) = self.quit.take() {
            unregister(quit);
        }
    }
}
