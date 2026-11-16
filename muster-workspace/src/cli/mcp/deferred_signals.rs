use std::{
    ffi::c_int,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use muster_os::signal_hold::SignalHold;
use signal_hook::{
    SigId,
    consts::{SIGHUP, SIGINT, SIGTERM},
    flag::register,
    low_level::{emulate_default_handler, unregister},
};

/// The signals a change to an agent's config must not be cut short by.
const DEFERRED: [c_int; 3] = [SIGINT, SIGTERM, SIGHUP];

/// Holds Ctrl-C, a terminate and a hangup while muster changes an agent's
/// config, so a step that puts back what it replaced always runs. Once it
/// drops, a signal that came meanwhile ends muster as it would have. An
/// RAII guard, so it has a hand-written constructor.
pub struct DeferredSignals {
    /// The holds that keep each signal from ending muster.
    holds: Vec<SignalHold>,
    /// Each signal's handler and the flag it raises.
    flags: Vec<(c_int, SigId, Arc<AtomicBool>)>,
}

impl DeferredSignals {
    /// Starts holding the signals. One that cannot be held, as when its
    /// handler cannot be registered, ends muster as before.
    pub fn begin() -> Self {
        let mut deferred = Self {
            holds: Vec::new(),
            flags: Vec::new(),
        };
        for signal in DEFERRED {
            let flag = Arc::new(AtomicBool::new(false));
            let Ok(id) = register(signal, Arc::clone(&flag)) else {
                continue;
            };
            match SignalHold::take(signal) {
                Ok(hold) => {
                    deferred.holds.push(hold);
                    deferred.flags.push((signal, id, flag));
                },
                Err(_) => {
                    unregister(id);
                },
            }
        }
        deferred
    }

    /// The first signal that came while held, if any.
    pub fn arrived(&self) -> Option<c_int> {
        self.flags
            .iter()
            .find(|(_, _, flag)| flag.load(Ordering::SeqCst))
            .map(|(signal, ..)| *signal)
    }
}

impl Drop for DeferredSignals {
    /// Stops holding the signals, then ends muster by one that came
    /// meanwhile.
    fn drop(&mut self) {
        let arrived = self.arrived();
        for (_, id, _) in self.flags.drain(..) {
            unregister(id);
        }
        self.holds.clear();
        if let Some(signal) = arrived {
            let _ = emulate_default_handler(signal);
        }
    }
}
