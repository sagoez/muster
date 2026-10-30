use std::{
    env,
    path::{Path, PathBuf},
    sync::Arc,
};

use super::{
    cli_ports::CliPorts, error::CliError, report::print_note,
    shell_registration::name_in_hooked_shell,
};
use crate::{
    adapter::{
        client::{ProjectTarget, ServiceAccess, ServiceClient, env_project, resolve_project},
        path::home_dir,
    },
    application::StackEditor,
    constants::{APP_NAME, MUSTER_PROJECT_ENV},
    domain::{
        port::{LocalState, StateChangeNotifier},
        project::Project,
        value::ProjectRoot,
    },
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

/// One run of the CLI: the global `--project` and the ports it runs on,
/// through which every command finds its state and its project the same way.
pub struct Invocation<'a> {
    /// The global `--project`, as typed.
    project: Option<String>,
    /// The adapters the composition root wired.
    ports: &'a dyn CliPorts,
    /// How the CLI reaches the service.
    service: Arc<dyn ServiceAccess>,
    /// Tells a running service of each stored change.
    notifier: Arc<dyn StateChangeNotifier>,
}

impl<'a> Invocation<'a> {
    /// The invocation with `--project` set to `project`, on `ports`.
    pub fn new(project: Option<String>, ports: &'a dyn CliPorts) -> Self {
        Self {
            project,
            ports,
            service: ports.service(),
            notifier: ports.change_notifier(),
        }
    }

    /// The adapters the composition root wired.
    pub fn ports(&self) -> &'a dyn CliPorts {
        self.ports
    }

    /// How the CLI reaches the service.
    pub fn service(&self) -> &dyn ServiceAccess {
        self.service.as_ref()
    }

    /// A client of this user's service, started or replaced as needed.
    ///
    /// # Errors
    /// Returns the [`CliError`] of a service that cannot be reached.
    pub fn connect(&self) -> Result<ServiceClient, CliError> {
        Ok(self.service.connect()?)
    }

    /// What tells a running service of each stored change.
    pub fn notifier(&self) -> &dyn StateChangeNotifier {
        self.notifier.as_ref()
    }

    /// muster's local state.
    ///
    /// # Errors
    /// Returns the [`CliError::Config`] of a database that cannot be opened.
    pub fn store(&self) -> Result<Arc<dyn LocalState>, CliError> {
        Ok(self.ports.open_state()?)
    }

    /// The use case an edit goes through: writes to `store`, then tells a
    /// running service, and names a project it registers in the hooked
    /// shell's record.
    pub fn editor<'s>(&'s self, store: &'s dyn LocalState) -> StackEditor<'s> {
        StackEditor::builder()
            .stacks(store)
            .settings(store)
            .notifier(self)
            .build()
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
    pub fn target(&self, store: &dyn LocalState) -> Result<ProjectTarget, CliError> {
        Ok(resolve_project(
            self.project.as_deref(),
            env_project(env::var_os(MUSTER_PROJECT_ENV).map(PathBuf::from)).as_deref(),
            &self.cwd()?,
            store,
        )?)
    }

    /// The registered project the command acts on: a folder named inside a
    /// project is that project.
    ///
    /// # Errors
    /// Returns [`CliError::NotAProject`] outside every project, or the
    /// [`CliError`] of a project that cannot be resolved.
    pub fn project(&self, store: &dyn LocalState) -> Result<Project, CliError> {
        Ok(self.target(store)?.within(store)?)
    }

    /// The project a command that edits a stack acts on, as [`Self::target`]
    /// finds it, after a note on stderr when `MUSTER_PROJECT` chose it over
    /// another project the current folder is in.
    ///
    /// # Errors
    /// Returns the [`CliError`] of a project that cannot be resolved.
    pub fn editing_target(&self, store: &dyn LocalState) -> Result<ProjectTarget, CliError> {
        let target = self.target(store)?;
        self.note_override(store, &target)?;
        Ok(target)
    }

    /// The project a command that may register its folder acts on, as
    /// [`Self::editing_target`] finds it, refusing a folder that cannot be a
    /// project.
    ///
    /// # Errors
    /// Returns [`CliError::Unregistrable`] for such a folder, or the
    /// [`CliError`] of a project that cannot be resolved.
    pub fn registering_target(&self, store: &dyn LocalState) -> Result<ProjectTarget, CliError> {
        Ok(self
            .editing_target(store)?
            .registrable(home_dir().as_deref(), store)?)
    }

    /// The registered project a command that edits a stack acts on, noted as
    /// [`Self::editing_target`] notes it; a folder named inside a project is
    /// that project.
    ///
    /// # Errors
    /// Returns [`CliError::NotAProject`] outside every project, or the
    /// [`CliError`] of a project that cannot be resolved.
    pub fn editing_project(&self, store: &dyn LocalState) -> Result<Project, CliError> {
        let project = self.target(store)?.within(store)?;
        self.note_override(store, &ProjectTarget::Registered(project.clone()))?;
        Ok(project)
    }

    /// Notes on stderr when `MUSTER_PROJECT` made `target` the project acted
    /// on rather than the one the current folder is in.
    ///
    /// # Errors
    /// Returns the [`CliError`] of a folder or store that cannot be read.
    fn note_override(
        &self,
        store: &dyn LocalState,
        target: &ProjectTarget,
    ) -> Result<(), CliError> {
        if let Some(here) = self.overridden(store, target)? {
            print_note(APP_NAME, &override_note(target, &here));
        }
        Ok(())
    }

    /// The registered project the current folder lies in, when
    /// `MUSTER_PROJECT` rather than `--project` made `target` another one.
    ///
    /// # Errors
    /// Returns the [`CliError`] of a folder or store that cannot be read.
    fn overridden(
        &self,
        store: &dyn LocalState,
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

impl StateChangeNotifier for Invocation<'_> {
    /// Also names the project in the record of the hooked shell this
    /// command runs straight from, as [`name_in_hooked_shell`] allows.
    fn project_registered(&self, root: &ProjectRoot) {
        self.notifier.project_registered(root);
        if let Ok(cwd) = self.cwd() {
            name_in_hooked_shell(self.ports, root, &cwd);
        }
    }

    fn stack_changed(&self, root: &ProjectRoot) {
        self.notifier.stack_changed(root);
    }

    fn settings_changed(&self) {
        self.notifier.settings_changed();
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::{super::fake_ports::FakePorts, *};
    use crate::{
        adapter::{
            client::ClientError,
            no_state_dir::NoStateDir,
            store::test_support::{fresh_store, spec},
        },
        domain::{config::ImportMode, port::StackReader, value::ProjectRoot},
    };

    /// Ports over a fresh scratch database, and the folder holding it.
    fn ports() -> (FakePorts, TempDir) {
        let (store, scratch) = fresh_store();
        (FakePorts::new(store), scratch)
    }

    /// An edit goes to the injected state and is told through the injected
    /// notifier, and reaching the service goes through the injected port, so
    /// no CLI path is wired to the user's real service.
    #[test]
    fn an_edit_and_the_service_go_through_the_injected_ports() {
        let (ports, scratch) = ports();
        let dir = scratch.path().join("shop");
        fs::create_dir_all(&dir).unwrap();
        let root = ProjectRoot::try_new(dir).unwrap();
        let invocation = Invocation::new(None, &ports);
        let store = invocation.store().unwrap();

        invocation
            .editor(store.as_ref())
            .import(&root, &[spec("web", "true")], ImportMode::Strict)
            .unwrap();

        assert!(ports.store().stack(&root).unwrap().is_some());
        assert_eq!(*ports.notifier().registered().lock().unwrap(), vec![
            root.clone()
        ]);
        assert_eq!(*ports.notifier().stacks().lock().unwrap(), vec![root]);
        assert!(ports.fake_service().told().is_empty());
        assert!(matches!(
            invocation.connect(),
            Err(CliError::Client(ClientError::NoStateDir(NoStateDir)))
        ));
        assert!(!invocation.service().stop().unwrap());
    }
}
