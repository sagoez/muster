use std::{
    fs,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
};

use getset::Getters;
use rustix::process::setsid;
use typed_builder::TypedBuilder;

use super::{
    config::{HolderConfig, error_path_beside, write_private_file},
    error::HolderError,
    holder_launcher::HolderLauncher,
    server::HolderServer,
};
use crate::{adapter::open_files::OpenFilesLimit, domain::pty::PtyError};

/// Hidden subcommand that runs one holder in the current process.
pub const HOLDER_SUBCOMMAND: &str = "__holder";
/// Flag naming the holder's spec file.
pub const SPEC_FLAG: &str = "--spec";

/// Launches each holder as `<binary> __holder --spec <json-file>`, detached
/// into its own session with no stdio, so it outlives the service and the
/// user's shell.
#[derive(Debug, Clone, Getters, TypedBuilder)]
pub struct ExecHolderLauncher {
    /// The muster executable to run.
    #[getset(get = "pub")]
    binary: PathBuf,
    /// The open-files soft limit the service started with, which each holder,
    /// and so its child, starts back at.
    #[getset(get = "pub")]
    open_files: OpenFilesLimit,
}

impl HolderLauncher for ExecHolderLauncher {
    /// Hands `config` over in an owner-only file beside the holder's private
    /// socket, then starts the detached holder that reads and deletes it.
    ///
    /// # Errors
    /// Returns a [`PtyError`] when the config cannot be written or the holder
    /// cannot be started.
    fn launch(&self, config: HolderConfig) -> Result<(), PtyError> {
        let path = config.spec_path();
        config.write_private(&path)?;
        let mut command = Command::new(&self.binary);
        command.arg(HOLDER_SUBCOMMAND).arg(SPEC_FLAG).arg(&path);
        match self.open_files.restore_in(detached(&mut command)).spawn() {
            Ok(holder) => {
                reap(holder);
                Ok(())
            },
            Err(error) => {
                let _ = fs::remove_file(&path);
                Err(error.into())
            },
        }
    }
}

/// Runs the holder handed the spec at `spec` until its child's exit is
/// collected. A holder has no stdio, so an error it fails with is also written
/// to an owner-only file beside the spec, where the launching runner reads it.
///
/// # Errors
/// Returns a [`HolderError`] when the spec cannot be read or the holder fails.
pub fn run_holder(spec: &Path) -> Result<(), HolderError> {
    let ran = HolderConfig::take_from(spec).and_then(HolderServer::run);
    if let Err(error) = &ran {
        let _ = write_private_file(&error_path_beside(spec), error.to_string().as_bytes());
    }
    ran
}

/// Makes `command` start in a new session with stdio closed off. `setsid`
/// also makes it a new process group's leader; asking std for a process group
/// as well would make that `setsid` fail, since std sets the group first.
pub(crate) fn detached(command: &mut Command) -> &mut Command {
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
pub(crate) fn reap(mut child: Child) {
    thread::spawn(move || {
        let _ = child.wait();
    });
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use crate::adapter::holder::reaped::Reaped;

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
        let child = Reaped(detached(&mut command).spawn().unwrap());
        let pid = child.0.id();

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
