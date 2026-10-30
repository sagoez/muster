use std::{
    io,
    sync::{
        Arc, Mutex, OnceLock, PoisonError,
        atomic::{AtomicBool, Ordering},
    },
};

use signal_hook::{consts::SIGINT, flag::register};

use super::interrupt_flags::Flags;
use crate::adapter::signal_hold::{SignalHold, catch_ending_signals};

/// The flags, registered once per process; `None` when registering failed,
/// which leaves `SIGINT` to end the process as usual.
static FLAGS: OnceLock<Option<Flags>> = OnceLock::new();

/// Turns Ctrl-C into a flag while a spinner or prompt is on the terminal, so
/// the command can close its frame and give the cursor back before it ends,
/// instead of being killed mid-frame. Dropping it restores the usual
/// ending on `SIGINT`. An RAII guard, so it has a hand-written constructor.
pub struct InterruptGuard {
    /// The flags it armed, unless they could not be registered.
    flags: Option<&'static Flags>,
    /// Keeps `SIGINT` from ending the process while the guard lives.
    hold: Option<SignalHold>,
}

impl InterruptGuard {
    /// Arms `SIGINT` as a flag until the guard drops.
    pub fn arm() -> Self {
        let flags = FLAGS.get_or_init(|| register_flags().ok()).as_ref();
        let hold = flags.and_then(|flags| {
            let mut armed = flags.armed.lock().unwrap_or_else(PoisonError::into_inner);
            if *armed == 0 {
                flags.interrupted.store(false, Ordering::SeqCst);
            }
            *armed += 1;
            SignalHold::take(SIGINT).ok()
        });
        Self { flags, hold }
    }

    /// Whether a `SIGINT` arrived since the guard was armed.
    pub fn interrupted(&self) -> bool {
        self.flags
            .is_some_and(|flags| flags.interrupted.load(Ordering::SeqCst))
    }
}

impl Drop for InterruptGuard {
    /// Lets a later `SIGINT` end the process again, once no guard is left.
    fn drop(&mut self) {
        if let Some(flags) = self.flags {
            let mut armed = flags.armed.lock().unwrap_or_else(PoisonError::into_inner);
            *armed = armed.saturating_sub(1);
            self.hold.take();
        }
    }
}

/// Catches the ending signals and registers the flag `SIGINT` sets.
///
/// # Errors
/// Returns the [`io::Error`] of a handler that could not be registered.
fn register_flags() -> io::Result<Flags> {
    catch_ending_signals()?;
    let flags = Flags {
        interrupted: Arc::new(AtomicBool::new(false)),
        armed: Mutex::new(0),
    };
    register(SIGINT, Arc::clone(&flags.interrupted))?;
    Ok(flags)
}

#[cfg(test)]
mod tests {
    use signal_hook::low_level::raise;

    use super::*;
    use crate::adapter::{
        child_group,
        cli::{error::CliError, presenter::asked},
    };

    /// A `SIGINT` while armed only sets the flag, a new guard starts clear,
    /// a nested guard leaves the outer one armed and interrupted, and a
    /// question is not put once an interrupt is pending.
    #[test]
    fn an_armed_guard_turns_sigint_into_a_flag() {
        let _alone = child_group::sweep_alone();
        let guard = InterruptGuard::arm();
        assert!(!guard.interrupted());
        raise(SIGINT).unwrap();
        assert!(guard.interrupted());
        drop(guard);
        assert!(!InterruptGuard::arm().interrupted());

        // A guard armed inside another neither clears its pending interrupt
        // nor disarms it when it drops: a later SIGINT still only sets the
        // flag. One test, as the flags are global.
        let outer = InterruptGuard::arm();
        raise(SIGINT).unwrap();
        let inner = InterruptGuard::arm();
        assert!(inner.interrupted(), "the pending interrupt survives");
        drop(inner);
        raise(SIGINT).unwrap();
        assert!(outer.interrupted());
        let put: Result<(), CliError> = asked(|| unreachable!("the question is never put"));
        assert!(matches!(put, Err(CliError::Cancelled)));
    }
}
