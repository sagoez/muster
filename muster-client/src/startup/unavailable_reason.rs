use std::{io, path::PathBuf, process::ExitStatus, time::Duration};

use humantime::format_duration;
use thiserror::Error;

/// Why the muster service could not be started or reached.
#[derive(Debug, Error)]
pub enum UnavailableReason {
    /// This binary's own path could not be found to start the service from.
    #[error("cannot find the muster binary to start it from: {0}")]
    NoBinary(#[source] io::Error),
    /// The state directory could not be created.
    #[error("cannot create the state directory {path}: {source}")]
    StateDir {
        /// The state directory.
        path: PathBuf,
        /// What failed.
        #[source]
        source: io::Error,
    },
    /// The start lock could not be opened or taken.
    #[error("cannot lock {path}: {source}")]
    StartLock {
        /// The lock file.
        path: PathBuf,
        /// What failed.
        #[source]
        source: io::Error,
    },
    /// Another command held the start lock for the whole wait.
    #[error("another muster command was still starting it after {}", format_duration(*.0))]
    StartLockBusy(Duration),
    /// The service binary could not be run.
    #[error("cannot run {binary} {subcommand}: {source}")]
    Spawn {
        /// The binary.
        binary: PathBuf,
        /// The hidden subcommand that runs the service.
        subcommand: &'static str,
        /// What failed.
        #[source]
        source: io::Error,
    },
    /// The service started by this command exited with a failure.
    #[error("it exited ({0}) while starting")]
    Exited(ExitStatus),
    /// The service did not answer in time after being started.
    #[error("it did not answer within {} of starting", format_duration(*.0))]
    NoAnswer(Duration),
    /// Another service holds the state directory and never answered.
    #[error(
        "another muster service holds the state directory and did not answer within {}",
        format_duration(*.0)
    )]
    HeldElsewhere(Duration),
    /// A service that was shutting down had still not gone.
    #[error("the running service was still shutting down after {}", format_duration(*.0))]
    StillExiting(Duration),
    /// A service of another version did not exit when asked to make way.
    #[error("the running service (version {theirs}) did not make way for version {ours}: {source}")]
    DidNotMakeWay {
        /// The running service's version.
        theirs: String,
        /// This binary's version.
        ours: &'static str,
        /// What failed.
        #[source]
        source: io::Error,
    },
}
