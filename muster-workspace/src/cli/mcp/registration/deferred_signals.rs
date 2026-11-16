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

    /// Stops holding the signals and returns the one that came while they
    /// were held. The holds go first, so a signal from then on ends muster
    /// by its own handler; only then are the flags read, after `released`
    /// runs, and unregistered, so no signal falls between the two and is
    /// lost.
    fn release(&mut self, released: impl FnOnce(&Self)) -> Option<c_int> {
        self.holds.clear();
        released(self);
        let arrived = self.arrived();
        for (_, id, _) in self.flags.drain(..) {
            unregister(id);
        }
        arrived
    }
}

impl Drop for DeferredSignals {
    /// Stops holding the signals, then ends muster by one that came
    /// meanwhile.
    fn drop(&mut self) {
        if let Some(signal) = self.release(|_| {}) {
            let _ = emulate_default_handler(signal);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A signal that lands while the holds are being given up is still
    /// seen, as the flags are read only after the holds are gone.
    #[test]
    fn a_signal_during_release_is_not_lost() {
        let mut deferred = DeferredSignals::begin();
        assert!(!deferred.flags.is_empty(), "the signals are held");

        let arrived = deferred.release(|held| {
            assert!(held.holds.is_empty(), "the holds go first");
            if let Some((_, _, flag)) = held.flags.first() {
                flag.store(true, Ordering::SeqCst);
            }
        });

        assert_eq!(arrived, Some(DEFERRED[0]));
        assert!(deferred.flags.is_empty());
    }
}
