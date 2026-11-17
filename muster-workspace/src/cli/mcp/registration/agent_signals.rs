use std::sync::{
    Arc,
    atomic::{AtomicI32, AtomicUsize, Ordering},
};

use muster_os::signal_hold::{SignalHold, is_ignored};
use signal_hook::{
    SigId,
    consts::{SIGHUP, SIGINT, SIGQUIT, SIGTERM},
    low_level::{register, unregister},
};

/// The signals that stop `muster mcp add`: Ctrl-C and Ctrl-\ from the
/// terminal, and a hangup or terminate, sent to muster alone or with it.
pub const STOPPING: [i32; 4] = [SIGINT, SIGQUIT, SIGHUP, SIGTERM];
/// The stopping signals that would otherwise end muster at once, through
/// its own handler, unless held; ending muster by one of these tells its
/// parent it was signalled.
pub const ENDING: [i32; 3] = [SIGINT, SIGHUP, SIGTERM];
/// The stopping signals a terminal sends to the agent's group as well, so
/// muster never passes them on itself.
const FROM_THE_TERMINAL: [i32; 2] = [SIGINT, SIGQUIT];
/// What [`AgentSignals`] records before any stopping signal arrived.
const NONE_RECEIVED: i32 = 0;

/// Keeps the stopping signals from ending muster while an agent's CLI runs,
/// and records which arrived and how many: a terminal sends Ctrl-C to the
/// agent too, which may catch it and carry on, so muster waits for the
/// agent's command to end and then stops before the next; a hangup or
/// terminate sent to muster alone it passes on to the agent; a second
/// stopping signal ends muster at once. A signal muster inherited as
/// ignored, as under `nohup`, stays ignored for it and for the agent;
/// every other is caught by a handler, never ignored, so the agent starts
/// with its default action. An RAII guard, so it has a hand-written
/// constructor.
pub struct AgentSignals {
    /// The holds that keep muster's own handler from ending it.
    _holds: Vec<SignalHold>,
    /// The last stopping signal that arrived, or [`NONE_RECEIVED`].
    received: Arc<AtomicI32>,
    /// How many stopping signals arrived.
    count: Arc<AtomicUsize>,
    /// The handlers recording them, unregistered on drop.
    handlers: Vec<SigId>,
}

impl AgentSignals {
    /// Catches and records the stopping signals not inherited as ignored
    /// until the guard drops; one that cannot be caught keeps its usual
    /// action.
    pub fn hold() -> Self {
        let received = Arc::new(AtomicI32::new(NONE_RECEIVED));
        let count = Arc::new(AtomicUsize::new(0));
        let caught = || STOPPING.into_iter().filter(|signal| !is_ignored(*signal));
        let handlers = caught()
            .filter_map(|signal| {
                let (received, count) = (Arc::clone(&received), Arc::clone(&count));
                // SAFETY: the action only stores to and adds to atomics,
                // which is async-signal-safe and allocates nothing.
                unsafe {
                    register(signal, move || {
                        received.store(signal, Ordering::SeqCst);
                        count.fetch_add(1, Ordering::SeqCst);
                    })
                }
                .ok()
            })
            .collect();
        Self {
            _holds: caught()
                .filter(|signal| ENDING.contains(signal))
                .filter_map(|signal| SignalHold::take(signal).ok())
                .collect(),
            received,
            count,
            handlers,
        }
    }

    /// The stopping signal that arrived last since the guard was taken, if
    /// any.
    pub fn received(&self) -> Option<i32> {
        match self.received.load(Ordering::SeqCst) {
            NONE_RECEIVED => None,
            signal => Some(signal),
        }
    }

    /// Whether more than one stopping signal arrived: the person asked
    /// twice, so muster ends at once.
    pub fn repeated(&self) -> bool {
        self.count.load(Ordering::SeqCst) > 1
    }

    /// Whether `signal` reaches muster alone, as no terminal sends it to the
    /// agent's group too, so muster passes it on.
    pub fn passed_on(signal: i32) -> bool {
        !FROM_THE_TERMINAL.contains(&signal)
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
