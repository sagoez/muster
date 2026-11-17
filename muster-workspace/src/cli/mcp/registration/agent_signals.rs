use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use muster_os::signal_hold::SignalHold;
use signal_hook::{
    SigId,
    consts::{SIGHUP, SIGINT, SIGQUIT, SIGTERM},
    flag::register_usize,
    low_level::unregister,
};

/// The signals that stop `muster mcp add`: Ctrl-C and Ctrl-\ from the
/// terminal, and a hangup or terminate, sent to muster alone or with it.
const STOPPING: [i32; 4] = [SIGINT, SIGQUIT, SIGHUP, SIGTERM];
/// The stopping signals that would otherwise end muster at once, through
/// its own handler, unless held.
const ENDING: [i32; 3] = [SIGINT, SIGHUP, SIGTERM];
/// What [`AgentSignals`] records before any stopping signal arrived.
const NONE_RECEIVED: usize = 0;

/// Keeps the stopping signals from ending muster while an agent's CLI runs,
/// and records which arrived: a terminal sends Ctrl-C to the agent too,
/// which may catch it and carry on, and a signal sent to muster alone never
/// reaches the agent, so muster waits for the agent's command to end and
/// then stops before the next. Each is caught by a handler, never ignored,
/// so the agent starts with its default action. An RAII guard, so it has a
/// hand-written constructor.
pub struct AgentSignals {
    /// The holds that keep muster's own handler from ending it.
    _holds: Vec<SignalHold>,
    /// The last stopping signal that arrived, or [`NONE_RECEIVED`].
    received: Arc<AtomicUsize>,
    /// The handlers recording them, unregistered on drop.
    handlers: Vec<SigId>,
}

impl AgentSignals {
    /// Catches and records the stopping signals until the guard drops; one
    /// that cannot be caught keeps its usual action.
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
            _holds: ENDING
                .into_iter()
                .filter_map(|signal| SignalHold::take(signal).ok())
                .collect(),
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
    /// Unregisters the recording handlers, and the holds let muster's own
    /// handler end it again. signal-hook keeps its handler for `SIGQUIT`,
    /// which then does nothing, for the little muster runs after the agent.
    fn drop(&mut self) {
        for handler in self.handlers.drain(..) {
            unregister(handler);
        }
    }
}
