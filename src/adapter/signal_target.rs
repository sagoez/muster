use std::io;

use rustix::{
    io::Errno,
    process::{
        Pid as RawPid, Signal, kill_process, kill_process_group, test_kill_process,
        test_kill_process_group,
    },
};

use crate::domain::value::Pid;

/// What a signal or a liveness probe reaches: one process, or the whole
/// process group a child leads, so backgrounded descendants are reached too.
/// A [`Pid`] is above init, so neither can ever name every process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalTarget {
    /// The one process.
    Process(Pid),
    /// Every process in the group the process leads.
    Group(Pid),
}

impl SignalTarget {
    /// Sends `signal` to this target.
    ///
    /// # Errors
    /// Returns the OS error when the target is gone or may not be signalled,
    /// or when its pid does not fit the platform's.
    pub fn send(self, signal: Signal) -> io::Result<()> {
        match self {
            Self::Process(pid) => kill_process(raw(pid)?, signal)?,
            Self::Group(pid) => kill_process_group(raw(pid)?, signal)?,
        }
        Ok(())
    }

    /// Whether anything this target names still exists. A process this user
    /// may not signal, or a probe that fails for any reason but the target's
    /// absence, counts as alive, so uncertainty never reports a process gone.
    pub fn is_alive(self) -> bool {
        let Ok(pid) = raw(self.pid()) else {
            return false;
        };
        let probe = match self {
            Self::Process(_) => test_kill_process(pid),
            Self::Group(_) => test_kill_process_group(pid),
        };
        !matches!(probe, Err(Errno::SRCH))
    }

    /// The pid this target is named by.
    fn pid(self) -> Pid {
        match self {
            Self::Process(pid) | Self::Group(pid) => pid,
        }
    }
}

/// `pid` as the platform's process id.
///
/// # Errors
/// Returns [`io::ErrorKind::InvalidInput`] for a pid the platform cannot
/// hold.
fn raw(pid: Pid) -> io::Result<RawPid> {
    i32::try_from(pid.into_inner())
        .ok()
        .and_then(RawPid::from_raw)
        .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidInput))
}

#[cfg(test)]
mod tests {
    use std::process::{self, Command};

    use super::*;

    /// This process is alive, and a child stays alive until it is killed
    /// and reaped.
    #[test]
    fn a_process_is_alive_until_it_is_killed_and_reaped() {
        assert!(SignalTarget::Process(Pid::try_new(process::id()).unwrap()).is_alive());
        let mut child = Command::new("sleep").arg("600").spawn().unwrap();
        let pid = Pid::try_new(child.id()).unwrap();
        assert!(SignalTarget::Process(pid).is_alive());

        SignalTarget::Process(pid).send(Signal::KILL).unwrap();
        child.wait().unwrap();

        assert!(!SignalTarget::Process(pid).is_alive());
        assert!(SignalTarget::Process(pid).send(Signal::KILL).is_err());
    }
}
