//! The signals that end a muster command: `SIGINT`, `SIGHUP` and `SIGTERM`.
//! Once caught, each kills the child groups [`super::child_group`] tracks,
//! then still ends the command as by default, unless something holds the
//! signal to handle it itself. One handler decides, so no other may end the
//! process on these signals.

use std::{
    ffi::c_int,
    io::{self, ErrorKind},
    sync::{
        OnceLock,
        atomic::{AtomicUsize, Ordering},
    },
};

use signal_hook::{
    consts::{SIGHUP, SIGINT, SIGTERM},
    low_level::{emulate_default_handler, register},
};

use super::child_group;

/// The signals that end muster unless held.
const ENDING_SIGNALS: [c_int; 3] = [SIGINT, SIGHUP, SIGTERM];

/// How many holds each of [`ENDING_SIGNALS`] has, by position.
static HOLDS: [AtomicUsize; ENDING_SIGNALS.len()] =
    [const { AtomicUsize::new(0) }; ENDING_SIGNALS.len()];
/// Whether catching the signals worked, once tried.
static CAUGHT: OnceLock<Result<(), ErrorKind>> = OnceLock::new();

/// Catches the ending signals, once per process. Until then they end it by
/// the kernel's default, which kills no child group. Only a command calls
/// this: the service and holders handle their own.
///
/// # Errors
/// Returns the failure to register a handler.
pub fn catch_ending_signals() -> io::Result<()> {
    (*CAUGHT.get_or_init(|| register_all().map_err(|error| error.kind()))).map_err(io::Error::from)
}

/// Registers the one handler for each ending signal.
///
/// # Errors
/// Returns the failure to register a handler.
fn register_all() -> io::Result<()> {
    for (index, signal) in ENDING_SIGNALS.into_iter().enumerate() {
        // SAFETY: the action only loads atomics, sends `kill` and re-raises
        // the signal with its default disposition, all async-signal-safe.
        unsafe { register(signal, move || end_unless_held(index, signal)) }?;
    }
    Ok(())
}

/// Kills every tracked child group, then ends the process by `signal`, at
/// `index` of [`ENDING_SIGNALS`], unless it is held. A held signal still
/// ends the helper calls in flight: what holds it ends the command its own
/// way, and a call it waits on must not run on to its bound first.
fn end_unless_held(index: usize, signal: c_int) {
    child_group::kill_all();
    if HOLDS[index].load(Ordering::SeqCst) == 0 {
        let _ = emulate_default_handler(signal);
    }
}

/// Keeps one ending signal from ending muster while it lives, for code that
/// handles the signal itself. An RAII guard, so it has a hand-written
/// constructor.
#[derive(Debug)]
pub struct SignalHold {
    /// The held signal's position in [`ENDING_SIGNALS`].
    index: usize,
}

impl SignalHold {
    /// Holds `signal`, one of `SIGINT`, `SIGHUP` or `SIGTERM`, catching the
    /// ending signals first.
    ///
    /// # Errors
    /// Returns [`ErrorKind::InvalidInput`] for any other signal, or the
    /// failure to catch them.
    pub fn take(signal: c_int) -> io::Result<Self> {
        let index = ENDING_SIGNALS
            .iter()
            .position(|ending| *ending == signal)
            .ok_or_else(|| io::Error::from(ErrorKind::InvalidInput))?;
        catch_ending_signals()?;
        HOLDS[index].fetch_add(1, Ordering::SeqCst);
        Ok(Self { index })
    }
}

impl Drop for SignalHold {
    /// Lets the signal end muster again once no hold is left.
    fn drop(&mut self) {
        HOLDS[self.index].fetch_sub(1, Ordering::SeqCst);
    }
}

#[cfg(test)]
mod tests {
    use signal_hook::consts::SIGWINCH;

    use super::*;

    /// Only an ending signal can be held, and a hold counts until it drops.
    #[test]
    fn only_an_ending_signal_is_held_and_each_hold_counts() {
        let refused = SignalHold::take(SIGWINCH).unwrap_err();
        assert_eq!(refused.kind(), ErrorKind::InvalidInput);

        let index = 2;
        assert_eq!(ENDING_SIGNALS[index], SIGTERM);
        let before = HOLDS[index].load(Ordering::SeqCst);
        let hold = SignalHold::take(SIGTERM).unwrap();
        assert_eq!(HOLDS[index].load(Ordering::SeqCst), before + 1);
        drop(hold);
        assert_eq!(HOLDS[index].load(Ordering::SeqCst), before);
    }
}
