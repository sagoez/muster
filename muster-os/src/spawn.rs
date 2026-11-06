use std::{
    os::unix::process::CommandExt,
    process::{Child, Command, Stdio},
    thread,
};

use muster_domain::pty::{PtyError, SpawnRequest};
use rustix::process::setsid;

/// Makes `command` start in a new session with stdio closed off. `setsid`
/// also makes it a new process group's leader; asking std for a process group
/// as well would make that `setsid` fail, since std sets the group first.
pub fn detached(command: &mut Command) -> &mut Command {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: the hook runs between fork and exec and only makes the `setsid`
    // system call, which is async-signal-safe, allocates nothing and touches
    // no memory.
    unsafe {
        command.pre_exec(|| {
            setsid()?;
            Ok(())
        })
    }
}

/// Collects a detached child's exit status whenever it ends, so a child that
/// finishes while its spawner runs on leaves no zombie behind.
pub fn reap(mut child: Child) {
    thread::spawn(move || {
        let _ = child.wait();
    });
}

/// Refuses a request whose configured working directory is missing or not a
/// directory, before anything is spawned.
///
/// # Errors
/// Returns [`PtyError::InvalidWorkingDir`] naming the bad directory.
pub fn ensure_working_dir(request: &SpawnRequest) -> Result<(), PtyError> {
    match request.working_dir() {
        Some(dir) if !dir.is_dir() => Err(PtyError::InvalidWorkingDir(dir.clone())),
        _ => Ok(()),
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::fs;

    use super::*;
    use crate::reaped::Reaped;

    /// A command that outlives the checks made on it.
    const LONG_RUNNER: &str = "sleep";
    /// How long the long runner would run unless killed.
    const LONG_RUNNER_SECONDS: &str = "30";
    /// Index of the process group id among the `/proc/<pid>/stat` fields after
    /// the command name.
    const PGRP_FIELD: usize = 2;
    /// Index of the session id among the fields after the command name.
    const SESSION_FIELD: usize = 3;

    #[test]
    fn a_detached_command_leads_its_own_session_and_process_group() {
        let mut command = Command::new(LONG_RUNNER);
        command.arg(LONG_RUNNER_SECONDS);
        let child = Reaped::new(detached(&mut command).spawn().unwrap());
        let pid = child.id();

        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).unwrap();
        let fields = stat
            .rsplit_once(") ")
            .unwrap()
            .1
            .split_whitespace()
            .collect::<Vec<_>>();
        assert_eq!(fields[PGRP_FIELD], pid.to_string());
        assert_eq!(fields[SESSION_FIELD], pid.to_string());
    }
}
