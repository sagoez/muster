use std::{
    env,
    path::{Path, PathBuf},
};

use super::{cli_ports::CliPorts, error::CliError};
use crate::{
    adapter::{
        client::{ProjectTarget, RunningServiceNotifier, env_project, resolve_project},
        store::SqliteStore,
    },
    application::StackEditor,
    constants::MUSTER_PROJECT_ENV,
    domain::project::Project,
};

/// The use case every CLI edit goes through: writes to `store`, then tells a
/// running service.
pub fn stack_editor(store: &SqliteStore) -> StackEditor<'_> {
    StackEditor::builder()
        .stacks(store)
        .settings(store)
        .notifier(&RunningServiceNotifier)
        .build()
}

/// One run of the CLI: the global `--project` and the ports it runs on,
/// through which every command finds its state and its project the same way.
pub struct Invocation<'a> {
    /// The global `--project`, as typed.
    project: Option<String>,
    /// The adapters the composition root wired.
    ports: &'a dyn CliPorts,
}

impl<'a> Invocation<'a> {
    /// The invocation with `--project` set to `project`, on `ports`.
    pub fn new(project: Option<String>, ports: &'a dyn CliPorts) -> Self {
        Self { project, ports }
    }

    /// The adapters the composition root wired.
    pub fn ports(&self) -> &'a dyn CliPorts {
        self.ports
    }

    /// muster's local state.
    ///
    /// # Errors
    /// Returns the [`CliError::Config`] of a database that cannot be opened.
    pub fn store(&self) -> Result<SqliteStore, CliError> {
        Ok(self.ports.open_state()?)
    }

    /// The folder the command runs in.
    ///
    /// # Errors
    /// Returns [`CliError::CurrentDir`] when it cannot be read.
    pub fn cwd(&self) -> Result<PathBuf, CliError> {
        env::current_dir().map_err(CliError::CurrentDir)
    }

    /// Whether `--project` or the environment muster gives its processes
    /// names the project, rather than the current folder.
    pub fn names_project(&self) -> bool {
        self.project.is_some()
            || env_project(env::var_os(MUSTER_PROJECT_ENV).map(PathBuf::from)).is_some()
    }

    /// The project the command acts on, registered or not, by the one rule
    /// every command follows.
    ///
    /// # Errors
    /// Returns the [`CliError`] of a project that cannot be resolved.
    pub fn target(&self, store: &SqliteStore) -> Result<ProjectTarget, CliError> {
        Ok(resolve_project(
            self.project.as_deref(),
            env_project(env::var_os(MUSTER_PROJECT_ENV).map(PathBuf::from)).as_deref(),
            &self.cwd()?,
            store,
        )?)
    }

    /// The registered project the command acts on.
    ///
    /// # Errors
    /// Returns [`CliError::NotAProject`] outside every project, or the
    /// [`CliError`] of a project that cannot be resolved.
    pub fn project(&self, store: &SqliteStore) -> Result<Project, CliError> {
        Ok(self.target(store)?.registered()?)
    }

    /// `path` as typed, read against the current directory.
    ///
    /// # Errors
    /// Returns [`CliError::CurrentDir`] when that cannot be read.
    pub fn in_cwd(&self, path: &Path) -> Result<PathBuf, CliError> {
        Ok(self.cwd()?.join(path))
    }
}
