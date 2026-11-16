use std::{
    env,
    path::{Path, PathBuf},
    sync::Arc,
};

use muster_application::StackEditor;
use muster_client::{ProjectTarget, ServiceAccess, ServiceClient, env_project, resolve_project};
use muster_domain::{
    config::WritePlace,
    constants::MUSTER_PROJECT_ENV,
    coordination::Author,
    port::{LocalState, StateChangeNotifier},
    project::Project,
    value::ProjectRoot,
};
use muster_os::path::home_dir;

use crate::cli::{
    command_line::cli_ports::CliPorts, failure::error::CliError,
    hook::shell_registration::name_in_hooked_shell, rows::report::print_note,
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

    /// The global `--project`, as typed, when it was given.
    pub fn project_flag(&self) -> Option<&str> {
        self.project.as_deref()
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
        self.project_named().is_some()
    }

    /// What names the project rather than the current folder, as written:
    /// `--project`, else the environment muster gives its processes; none
    /// when neither does.
    pub fn project_named(&self) -> Option<String> {
        self.project
            .clone()
            .or_else(|| self.env_project().map(|root| root.display().to_string()))
    }

    /// The project the command acts on, registered or not, by the one rule
    /// every command follows.
    ///
    /// # Errors
    /// Returns the [`CliError`] of a project that cannot be resolved.
    pub fn target(&self, store: &dyn LocalState) -> Result<ProjectTarget, CliError> {
        Ok(resolve_project(
            self.project.as_deref(),
            self.env_project().as_deref(),
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

    /// Where a shared write by `by`, such as `note set`, goes, as
    /// [`ProjectTarget::write_place`] finds it from the folder `--project`
    /// names or the current one, noted as [`Self::editing_target`] notes it.
    /// `$MUSTER_PROJECT` names a project to write to, but never a folder to
    /// register.
    ///
    /// # Errors
    /// Returns [`CliError::NotAProject`] for a folder `$MUSTER_PROJECT` names
    /// outside every project, [`CliError::NoRepository`] for one in no
    /// project and no repository, [`CliError::Unregistrable`] for a
    /// repository root that cannot be a project, or the [`CliError`] of a
    /// project that cannot be resolved.
    pub fn write_place(&self, store: &dyn LocalState, by: &Author) -> Result<WritePlace, CliError> {
        let target = self.editing_target(store)?;
        if self.project.is_none() && self.env_project().is_some() {
            return Ok(target.within(store)?.root().clone().into());
        }
        Ok(target.write_place(by, home_dir().as_deref(), store)?)
    }

    /// The root `$MUSTER_PROJECT` names, when it is set.
    fn env_project(&self) -> Option<PathBuf> {
        env_project(env::var_os(MUSTER_PROJECT_ENV).map(PathBuf::from))
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
            print_note(&override_note(target, &here));
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
        if self.project.is_some() || self.env_project().is_none() {
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

    use muster_client::ClientError;
    use muster_domain::{
        config::ImportMode,
        coordination::{NoteBody, ScratchpadKey},
        port::{ProjectStore, StackReader},
    };
    use muster_os::no_state_dir::NoStateDir;
    use muster_store::test_support::{fresh_store, spec};
    use tempfile::TempDir;

    use super::{super::fake_ports::FakePorts, *};

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

    /// A shared write from `--project`'s folder in a repository registers
    /// the repository root through the injected notifier, once; the folder
    /// inside it then writes to that project.
    #[test]
    fn a_shared_write_registers_the_repository_root_once() {
        let (ports, scratch) = ports();
        let dir = scratch.path().canonicalize().unwrap().join("shop");
        fs::create_dir_all(dir.join(".git")).unwrap();
        fs::create_dir_all(dir.join("web")).unwrap();
        let root = ProjectRoot::try_new(dir.clone()).unwrap();
        let at = |folder: &Path| Invocation::new(Some(folder.display().to_string()), &ports);
        let store = at(&dir).store().unwrap();
        let note = |invocation: &Invocation<'_>| {
            let place = invocation
                .write_place(store.as_ref(), &Author::human())
                .unwrap();
            invocation
                .editor(store.as_ref())
                .share(|| {
                    store.set_scratchpad(
                        &place,
                        &Author::human(),
                        &ScratchpadKey::try_new("plan").unwrap(),
                        &NoteBody::try_new("ship").unwrap(),
                    )
                })
                .unwrap()
        };

        let first = note(&at(&dir.join("web")));
        let again = note(&at(&dir.join("web")));

        assert_eq!(
            first.registered().as_ref().map(|project| project.root()),
            Some(&root)
        );
        assert!(again.registered().is_none());
        assert_eq!(ports.store().projects().unwrap().len(), 1);
        assert_eq!(*ports.notifier().registered().lock().unwrap(), vec![root]);
    }
}
