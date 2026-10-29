use thiserror::Error;

use crate::{
    adapter::{client::ClientError, no_state_dir::NoStateDir},
    domain::{config::ConfigError, occupancy::ShellRegistryError, value::Pid},
};

/// Why a shell's report wrote nothing. The hook never shows it: the report
/// runs detached at a prompt, and the next prompt reports again.
#[derive(Debug, Error)]
pub enum ShellReportSkip {
    /// The platform offers no state directory to keep shell files in.
    #[error(transparent)]
    NoStateDir(#[from] NoStateDir),
    /// No live process runs as the pid the hook reported.
    #[error("no shell runs as pid {0}")]
    UnknownShell(Pid),
    /// The shell's stored report is as new as this one or newer.
    #[error("a report as new or newer is already stored")]
    Stale,
    /// The database could not be read.
    #[error(transparent)]
    State(#[from] ConfigError),
    /// The service could not be reached, or answered something else.
    #[error(transparent)]
    Service(#[from] ClientError),
    /// The shell's record or notes could not be written.
    #[error(transparent)]
    Registry(#[from] ShellRegistryError),
}
