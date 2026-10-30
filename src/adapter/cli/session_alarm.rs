use std::{
    cell::RefCell,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicBool, AtomicI32, Ordering},
    },
};

use super::ending_signal::EndingSignal;

/// The stored signal number while no ending signal arrived.
const NO_SIGNAL: i32 = 0;

thread_local! {
    /// The alarm of the session this thread helps, when it is a helper.
    static SESSION_HELPER: RefCell<Option<Arc<SessionAlarm>>> = const { RefCell::new(None) };
}

/// What an attach session's helper threads and signal listener report
/// besides its inbox, which may be full: an ending signal, a helper that
/// failed, and the message of a helper's panic, held until the terminal is
/// given back.
#[derive(Debug, Default)]
pub struct SessionAlarm {
    /// The number of the ending signal that arrived, or [`NO_SIGNAL`].
    ending: AtomicI32,
    /// Whether a helper thread failed.
    helper_failed: AtomicBool,
    /// A helper's panic message, printed once the terminal is back.
    panic_message: Mutex<Option<String>>,
}

impl SessionAlarm {
    /// Records that `signal` arrived.
    pub fn record_ending(&self, signal: EndingSignal) {
        self.ending.store(signal.number(), Ordering::SeqCst);
    }

    /// The ending signal that arrived, if any.
    pub fn ending(&self) -> Option<EndingSignal> {
        match self.ending.load(Ordering::SeqCst) {
            NO_SIGNAL => None,
            number => EndingSignal::from_number(number),
        }
    }

    /// Records that a helper thread failed.
    pub fn record_helper_failure(&self) {
        self.helper_failed.store(true, Ordering::SeqCst);
    }

    /// Whether a helper thread failed.
    pub fn helper_failed(&self) -> bool {
        self.helper_failed.load(Ordering::SeqCst)
    }

    /// Holds a helper's panic `message` for after the session.
    pub fn hold_panic_message(&self, message: String) {
        *self
            .panic_message
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(message);
    }

    /// The held panic message, taken.
    pub fn take_panic_message(&self) -> Option<String> {
        self.panic_message
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

/// Marks the current thread as a helper of the session `alarm` belongs to.
pub(super) fn mark_session_helper(alarm: Arc<SessionAlarm>) {
    SESSION_HELPER.with_borrow_mut(|helper| *helper = Some(alarm));
}

/// Whether the current thread is a session's helper.
pub(super) fn on_session_helper() -> bool {
    SESSION_HELPER.with_borrow(Option::is_some)
}

/// For the panic hook: holds `message` for its session when the panic is on
/// one of a session's helper threads, which would otherwise print it over
/// the live screen. Answers whether it was held.
pub fn hold_helper_panic(message: String) -> bool {
    SESSION_HELPER.with_borrow(|helper| {
        helper
            .as_ref()
            .map(|alarm| alarm.hold_panic_message(message))
            .is_some()
    })
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    /// A panic on a helper thread is held for its session; one on any other
    /// thread is not.
    #[test]
    fn a_helper_panic_is_held_for_its_session() {
        let alarm = Arc::new(SessionAlarm::default());
        let helper = Arc::clone(&alarm);
        let held = thread::spawn(move || {
            mark_session_helper(helper);
            hold_helper_panic("boom".to_string())
        })
        .join()
        .unwrap();

        assert!(held);
        assert_eq!(alarm.take_panic_message().as_deref(), Some("boom"));
        assert!(
            !thread::spawn(|| hold_helper_panic("elsewhere".to_string()))
                .join()
                .unwrap()
        );
    }
}
