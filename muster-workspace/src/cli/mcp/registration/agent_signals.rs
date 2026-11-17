use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use muster_os::signal_hold::SignalHold;
use signal_hook::{
    SigId,
    consts::{SIGINT, SIGQUIT},
    flag::register_usize,
    low_level::unregister,
};

/// The signals a person sends from the terminal to stop `muster mcp add`.
const STOPPING: [i32; 2] = [SIGINT, SIGQUIT];
/// What [`AgentSignals`] records before any stopping signal arrived.
const NONE_RECEIVED: usize = 0;

/// Keeps `SIGINT` and `SIGQUIT` from ending muster while an agent's CLI
/// runs in the foreground, and records which arrived: the terminal sends
/// them to the agent too, which may catch them and carry on, so muster
/// waits for it and then stops before the next command. Both are caught by
/// a handler, never ignored, so the agent starts with their default action.
/// An RAII guard, so it has a hand-written constructor.
pub struct AgentSignals {
    /// The hold on `SIGINT`, so muster's own handler does not end it.
    _interrupt: Option<SignalHold>,
    /// The last stopping signal that arrived, or [`NONE_RECEIVED`].
    received: Arc<AtomicUsize>,
    /// The handlers recording them, unregistered on drop.
    handlers: Vec<SigId>,
}

impl AgentSignals {
    /// Catches and records both signals until the guard drops; one that
    /// cannot be caught keeps its usual action.
    pub fn hold() -> Self {
        let received = Arc::new(AtomicUsize::new(NONE_RECEIVED));
        let handlers = STOPPING
            .into_iter()
            .filter_map(|signal| {
                let value = usize::try_from(signal).ok()?;
                register_usize(signal, Arc::clone(&received), value).ok()
            })
            .collect();
        Self {
            _interrupt: SignalHold::take(SIGINT).ok(),
            received,
            handlers,
        }
    }

    /// The stopping signal that arrived since the guard was taken, if any.
    pub fn received(&self) -> Option<i32> {
        match self.received.load(Ordering::SeqCst) {
            NONE_RECEIVED => None,
            signal => i32::try_from(signal).ok(),
        }
    }
}

impl Drop for AgentSignals {
    /// Unregisters the recording handlers, and the hold lets `SIGINT` end
    /// muster again. signal-hook keeps its handler for `SIGQUIT`, which then
    /// does nothing, for the little muster runs after the agent.
    fn drop(&mut self) {
        for handler in self.handlers.drain(..) {
            unregister(handler);
        }
    }
}
