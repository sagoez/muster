use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use getset::Getters;
use muster_domain::{port::ProcessRunner, pty::PtyError};
use muster_os::{
    open_files::OpenFilesLimit,
    spawn::{detached, reap},
};
use typed_builder::TypedBuilder;

use super::{
    config::{HolderConfig, error_path_beside, write_private_file},
    error::HolderError,
    holder_launcher::HolderLauncher,
    server::HolderServer,
};

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
/// `runner` spawns the child.
///
/// # Errors
/// Returns a [`HolderError`] when the spec cannot be read or the holder fails.
pub fn run_holder(spec: &Path, runner: &dyn ProcessRunner) -> Result<(), HolderError> {
    let ran = HolderConfig::take_from(spec).and_then(|config| HolderServer::run(config, runner));
    if let Err(error) = &ran {
        let _ = write_private_file(&error_path_beside(spec), error.to_string().as_bytes());
    }
    ran
}
