use std::{
    os::unix::process::{CommandExt, ExitStatusExt},
    process::{Child, Command},
    thread,
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, unbounded};
use libc::{SIG_IGN, SIGINT, SIGTERM, signal};
use rustix::process::Signal;

use crate::{adapter::signal_target::SignalTarget, domain::value::Pid};

/// What an [`Orphan`] runs: a long sleep it never finishes on its own.
const ORPHAN_PROGRAM: &str = "sleep";
/// The orphan's sleep, in seconds, far past any test.
const ORPHAN_SECONDS: &str = "600";
/// Longest an orphan's drop waits for it to be gone.
const ORPHAN_REAP_WAIT: Duration = Duration::from_secs(10);
/// Interval between checks that a killed orphan is gone.
const ORPHAN_POLL: Duration = Duration::from_millis(10);

/// A real process in its own process group that ignores the graceful stop
/// signals, standing in for a child left running under a holder the service
/// cannot reach. By default a reaper thread collects it, so a killed orphan is
/// gone rather than a zombie, and reports the signal that ended it. Dropping
/// it kills the group, also on a panic.
pub(crate) struct Orphan {
    pid: Pid,
    /// The signal that ended a reaped orphan, once it ended.
    ended_by: Option<Receiver<Option<i32>>>,
    /// An orphan nobody reaps, which stays a zombie once killed until dropped.
    unreaped: Option<Child>,
}

impl Orphan {
    /// Starts an orphan that is reaped as soon as it dies.
    pub(crate) fn spawn() -> Self {
        let mut child = Self::command().spawn().unwrap();
        let pid = Pid::try_new(child.id()).unwrap();
        let (sender, ended_by) = unbounded();
        thread::spawn(move || {
            let signal = child.wait().ok().and_then(|status| status.signal());
            let _ = sender.send(signal);
        });
        Self {
            pid,
            ended_by: Some(ended_by),
            unreaped: None,
        }
    }

    /// Starts an orphan nobody reaps, so once killed it lingers as a zombie
    /// whose pid still exists.
    pub(crate) fn unreaped() -> Self {
        let child = Self::command().spawn().unwrap();
        Self {
            pid: Pid::try_new(child.id()).unwrap(),
            ended_by: None,
            unreaped: Some(child),
        }
    }

    /// The orphan's command: a long sleep in its own process group, ignoring
    /// SIGINT and SIGTERM.
    fn command() -> Command {
        let mut command = Command::new(ORPHAN_PROGRAM);
        command.arg(ORPHAN_SECONDS).process_group(0);
        // SAFETY: the hook runs between fork and exec and only resets two
        // signal dispositions with `signal`, which is async-signal-safe and
        // allocates nothing; rustix offers no safe way to set one.
        unsafe {
            command.pre_exec(|| {
                signal(SIGINT, SIG_IGN);
                signal(SIGTERM, SIG_IGN);
                Ok(())
            });
        }
        command
    }

    /// The orphan's process id, which also names its process group.
    pub(crate) fn pid(&self) -> Pid {
        self.pid
    }

    /// Whether the orphan still exists.
    pub(crate) fn alive(&self) -> bool {
        SignalTarget::Process(self.pid).is_alive()
    }

    /// Sends `signal` to a reaped orphan and returns the signal that ended
    /// it, waiting at most [`ORPHAN_REAP_WAIT`]. A signal something else sent
    /// first shows up as that signal instead.
    pub(crate) fn end_with(&self, signal: Signal) -> Option<i32> {
        let _ = SignalTarget::Process(self.pid).send(signal);
        self.ended_by
            .as_ref()?
            .recv_timeout(ORPHAN_REAP_WAIT)
            .ok()
            .flatten()
    }
}

impl Drop for Orphan {
    /// Kills the orphan's group and waits, bounded, until it is gone.
    fn drop(&mut self) {
        let _ = SignalTarget::Group(self.pid).send(Signal::KILL);
        if let Some(mut child) = self.unreaped.take() {
            let _ = child.wait();
            return;
        }
        let started = Instant::now();
        while self.alive() && started.elapsed() < ORPHAN_REAP_WAIT {
            thread::sleep(ORPHAN_POLL);
        }
    }
}
