//! An agent's CLI run as muster's child: a terminate or hangup sent to
//! muster meanwhile is passed on to it, and muster waits on, so the agent
//! is never left mid-write without anyone seeing how it ended. Forwarding
//! stops once the child has exited but before it is reaped, so no signal
//! ever reaches a pid the system could have given to another process.

use std::{
    ffi::c_int,
    io::{self, Read},
    process::{Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicI32, Ordering},
    },
    thread::{self, JoinHandle},
};

use getset::Getters;
use muster_os::signal_hold::SignalHold;
use rustix::{
    io::Errno,
    process::{Pid, Signal, WaitId, WaitIdOptions, kill_process, waitid},
};
use signal_hook::{
    consts::{SIGHUP, SIGINT, SIGTERM},
    iterator::Signals,
};
use typed_builder::TypedBuilder;

/// The signals held while the agent runs. A terminate or hangup is passed
/// on; a Ctrl-C the terminal already sends the agent too, so muster only
/// notes it.
const HELD: [c_int; 3] = [SIGTERM, SIGHUP, SIGINT];
/// Stands for no signal received.
const NO_SIGNAL: c_int = 0;

/// How the agent's CLI ended, what it printed when captured, and the stop
/// signal muster was sent while it ran.
#[derive(Debug, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct AgentRun {
    /// How the agent ended.
    status: ExitStatus,
    /// Its standard output, when captured.
    stdout: Vec<u8>,
    /// Its standard error, when captured.
    stderr: Vec<u8>,
    /// The stop muster was sent while waiting, if any: a terminate or
    /// hangup counts only when passed on to the agent still running; a
    /// Ctrl-C, which the terminal sends the agent itself, always counts.
    stop: Option<c_int>,
}

/// Spawns `command` and waits for it, holding [`HELD`] the whole time and
/// passing a terminate or hangup on to the child while it runs; its output
/// is read when `capture`, else passed through.
///
/// # Errors
/// Returns the I/O error of holding the signals, spawning or waiting.
pub fn run_forwarding(command: &mut Command, capture: bool) -> io::Result<AgentRun> {
    let holds = HELD
        .into_iter()
        .map(SignalHold::take)
        .collect::<io::Result<Vec<_>>>()?;
    let mut signals = Signals::new(HELD)?;
    let handle = signals.handle();
    if capture {
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            handle.close();
            return Err(error);
        },
    };
    let stdout = child.stdout.take().map(drained);
    let stderr = child.stderr.take().map(drained);
    let pid = Pid::from_child(&child);
    let stop = Arc::new(AtomicI32::new(NO_SIGNAL));
    let seen = Arc::clone(&stop);
    let forwarder = thread::spawn(move || {
        for signal in signals.forever() {
            match passed_on(signal) {
                Some(passed) if !exited(pid) => {
                    let _ = kill_process(pid, passed);
                    seen.store(signal, Ordering::SeqCst);
                },
                Some(_) => {},
                None => seen.store(signal, Ordering::SeqCst),
            }
        }
    });
    let exit = await_exit(pid);
    handle.close();
    let _ = forwarder.join();
    let status = child.wait();
    drop(holds);
    exit?;
    let stop = stop.load(Ordering::SeqCst);
    Ok(AgentRun::builder()
        .status(status?)
        .stdout(joined(stdout))
        .stderr(joined(stderr))
        .stop((stop != NO_SIGNAL).then_some(stop))
        .build())
}

/// Blocks until the child `pid` has exited, leaving it unreaped so its pid
/// stays its own.
///
/// # Errors
/// Returns the I/O error of waiting.
fn await_exit(pid: Pid) -> io::Result<()> {
    loop {
        match waitid(
            WaitId::Pid(pid),
            WaitIdOptions::EXITED | WaitIdOptions::NOWAIT,
        ) {
            Ok(_) => return Ok(()),
            Err(Errno::INTR) => {},
            Err(error) => return Err(error.into()),
        }
    }
}

/// Whether the child `pid` has already exited, looked at without reaping.
fn exited(pid: Pid) -> bool {
    matches!(
        waitid(
            WaitId::Pid(pid),
            WaitIdOptions::EXITED | WaitIdOptions::NOWAIT | WaitIdOptions::NOHANG
        ),
        Ok(Some(_))
    )
}

/// Reads all of `stream` on a thread of its own, so a child writing much
/// never blocks on a full pipe.
fn drained(mut stream: impl Read + Send + 'static) -> JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stream.read_to_end(&mut bytes);
        bytes
    })
}

/// What a reader thread read; nothing when there was none.
fn joined(reader: Option<JoinHandle<Vec<u8>>>) -> Vec<u8> {
    reader
        .and_then(|reader| reader.join().ok())
        .unwrap_or_default()
}

/// The signal `signal` is passed on to the agent as; none for a Ctrl-C.
fn passed_on(signal: c_int) -> Option<Signal> {
    match signal {
        SIGTERM => Some(Signal::TERM),
        SIGHUP => Some(Signal::HUP),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shell the tests run children with.
    const SHELL: &str = "sh";

    /// A terminate and a hangup are passed on as themselves; a Ctrl-C,
    /// which already reached the agent, is not.
    #[test]
    fn only_a_terminate_or_hangup_is_passed_on() {
        assert_eq!(passed_on(SIGTERM), Some(Signal::TERM));
        assert_eq!(passed_on(SIGHUP), Some(Signal::HUP));
        assert_eq!(passed_on(SIGINT), None);
    }

    /// A child that has exited is seen as exited while still unreaped, so
    /// its pid is never signalled once it could be another's, and the run
    /// reports how it ended and what it printed.
    #[test]
    fn an_exited_child_is_seen_before_it_is_reaped() {
        let run =
            run_forwarding(Command::new(SHELL).args(["-c", "echo said; exit 3"]), true).unwrap();
        assert_eq!(run.status().code(), Some(3));
        assert_eq!(run.stdout(), b"said\n");
        assert_eq!(*run.stop(), None);

        let mut done = Command::new(SHELL).args(["-c", "exit 0"]).spawn().unwrap();
        let pid = Pid::from_child(&done);
        await_exit(pid).unwrap();
        assert!(exited(pid), "exited, and still unreaped");
        assert!(done.wait().unwrap().success());
    }
}
