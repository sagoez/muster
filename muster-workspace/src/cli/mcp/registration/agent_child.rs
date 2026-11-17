//! An agent's CLI run as muster's child: a terminate or hangup sent to
//! muster meanwhile is passed on to it, and muster waits on, so the agent
//! is never left mid-write without anyone seeing how it ended.

use std::{
    ffi::c_int,
    io,
    process::{Child, Command},
    sync::{
        Arc,
        atomic::{AtomicI32, Ordering},
    },
    thread,
};

use muster_os::signal_hold::SignalHold;
use rustix::process::{Pid, Signal, kill_process};
use signal_hook::{
    consts::{SIGHUP, SIGINT, SIGTERM},
    iterator::Signals,
};

use crate::cli::session::signals::ending_signal::EndingSignal;

/// The signals held while the agent runs. A terminate or hangup is passed
/// on; a Ctrl-C the terminal already sends the agent too, so muster only
/// keeps it from ending muster first.
const HELD: [c_int; 3] = [SIGTERM, SIGHUP, SIGINT];
/// Stands for no signal received.
const NO_SIGNAL: c_int = 0;

/// Spawns `command` and waits for it with `wait`, holding [`HELD`] the whole
/// time and passing a terminate or hangup on to the child. Returns what
/// `wait` gave, and the last signal passed on, if any.
///
/// # Errors
/// Returns the I/O error of holding the signals, spawning or waiting.
pub fn run_forwarding<T>(
    command: &mut Command,
    wait: impl FnOnce(Child) -> io::Result<T>,
) -> io::Result<(T, Option<EndingSignal>)> {
    let holds = HELD
        .into_iter()
        .map(SignalHold::take)
        .collect::<io::Result<Vec<_>>>()?;
    let mut signals = Signals::new(HELD)?;
    let handle = signals.handle();
    let child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            handle.close();
            return Err(error);
        },
    };
    let pid = Pid::from_child(&child);
    let sent = Arc::new(AtomicI32::new(NO_SIGNAL));
    let seen = Arc::clone(&sent);
    let forwarder = thread::spawn(move || {
        for signal in signals.forever() {
            if let Some(passed) = passed_on(signal) {
                let _ = kill_process(pid, passed);
                seen.store(signal, Ordering::SeqCst);
            }
        }
    });
    let waited = wait(child);
    handle.close();
    let _ = forwarder.join();
    drop(holds);
    Ok((
        waited?,
        EndingSignal::from_number(sent.load(Ordering::SeqCst)),
    ))
}

/// The signal `signal` is passed on to the agent as; none for a Ctrl-C.
fn passed_on(signal: c_int) -> Option<Signal> {
    EndingSignal::from_number(signal).map(|ending| match ending {
        EndingSignal::Terminate => Signal::TERM,
        EndingSignal::Hangup => Signal::HUP,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A terminate and a hangup are passed on as themselves; a Ctrl-C,
    /// which already reached the agent, is not.
    #[test]
    fn only_a_terminate_or_hangup_is_passed_on() {
        assert_eq!(passed_on(SIGTERM), Some(Signal::TERM));
        assert_eq!(passed_on(SIGHUP), Some(Signal::HUP));
        assert_eq!(passed_on(SIGINT), None);
    }
}
