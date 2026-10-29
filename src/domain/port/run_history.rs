use crate::domain::{
    config::ConfigError,
    process::EndedRun,
    value::{ProcessName, ProjectRoot},
};

/// Driven port: how each process's last run ended on its own, kept while it
/// does not run, so a service started later shows it crashed or exited with
/// its last exit. Removing the process removes its record.
pub trait RunHistory: Send + Sync {
    /// The recorded ends of the processes of the project at `root`; none when
    /// it is not registered.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the store cannot be read.
    fn ended_runs(&self, root: &ProjectRoot) -> Result<Vec<EndedRun>, ConfigError>;

    /// Records how the run of `run`'s process of the project at `root` ended,
    /// replacing what was recorded. A process no longer stored is skipped.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the store cannot be written.
    fn record_end(&self, root: &ProjectRoot, run: &EndedRun) -> Result<(), ConfigError>;

    /// Forgets how `process` of the project at `root` last ended, once it
    /// runs again or was stopped on request.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the store cannot be written.
    fn clear_end(&self, root: &ProjectRoot, process: &ProcessName) -> Result<(), ConfigError>;
}
