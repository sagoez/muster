use std::{
    env,
    path::{Path, PathBuf},
};

use super::{cli_ports::CliPorts, error::CliError, report::print_note};
use crate::{
    adapter::{
        client::{ProjectTarget, RunningServiceNotifier, env_project, resolve_project},
        store::SqliteStore,
    },
    application::StackEditor,
    constants::{APP_NAME, MUSTER_PROJECT_ENV},
    domain::{port::ProjectStore, project::Project},
};

/// Between the project acted on and the one the current folder is in.
const NOT_HERE: &str = "acting on it, not on";
/// Follows the name of the project the current folder is in.
const HERE: &str = "which this folder is in";

/// The note that `MUSTER_PROJECT` made a command act on `target` rather than
/// `here`, the project the current folder lies in.
fn override_note(target: &ProjectTarget, here: &Project) -> String {
    let root = target.root().as_ref().display();
    let named = match target {
        ProjectTarget::Registered(project) => format!("{} ({root})", project.name()),
        ProjectTarget::Unregistered(_) => root.to_string(),
    };
    format!(
        "{MUSTER_PROJECT_ENV} names {named}; {NOT_HERE} {}, {HERE}",
        here.name()
    )
}

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

    /// The project a command that edits a stack acts on, as [`Self::target`]
    /// finds it, after a note on stderr when `MUSTER_PROJECT` chose it over
    /// another project the current folder is in.
    ///
    /// # Errors
    /// Returns the [`CliError`] of a project that cannot be resolved.
    pub fn editing_target(&self, store: &SqliteStore) -> Result<ProjectTarget, CliError> {
        let target = self.target(store)?;
        if let Some(here) = self.overridden(store, &target)? {
            print_note(APP_NAME, &override_note(&target, &here));
        }
        Ok(target)
    }

    /// The registered project a command that edits a stack acts on, noted as
    /// [`Self::editing_target`] notes it.
    ///
    /// # Errors
    /// Returns [`CliError::NotAProject`] outside every project, or the
    /// [`CliError`] of a project that cannot be resolved.
    pub fn editing_project(&self, store: &SqliteStore) -> Result<Project, CliError> {
        Ok(self.editing_target(store)?.registered()?)
    }

    /// The registered project the current folder lies in, when
    /// `MUSTER_PROJECT` rather than `--project` made `target` another one.
    ///
    /// # Errors
    /// Returns the [`CliError`] of a folder or store that cannot be read.
    fn overridden(
        &self,
        store: &SqliteStore,
        target: &ProjectTarget,
    ) -> Result<Option<Project>, CliError> {
        if self.project.is_some()
            || env_project(env::var_os(MUSTER_PROJECT_ENV).map(PathBuf::from)).is_none()
        {
            return Ok(None);
        }
        Ok(store
            .project_containing(&self.cwd()?)?
            .filter(|here| here.root() != target.root()))
    }

    /// `path` as typed, read against the current directory.
    ///
    /// # Errors
    /// Returns [`CliError::CurrentDir`] when that cannot be read.
    pub fn in_cwd(&self, path: &Path) -> Result<PathBuf, CliError> {
        Ok(self.cwd()?.join(path))
    }
}
