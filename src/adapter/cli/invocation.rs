use std::{
    env,
    path::{Path, PathBuf},
    sync::Arc,
};

use super::{cli_ports::CliPorts, error::CliError, report::print_note};
use crate::{
    adapter::client::{
        ProjectTarget, RunningServiceNotifier, ServiceAccess, ServiceClient, env_project,
        resolve_project,
    },
    application::StackEditor,
    constants::{APP_NAME, MUSTER_PROJECT_ENV},
    domain::{
        port::{LocalState, StateChangeNotifier},
        project::Project,
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
    notifier: RunningServiceNotifier,
}

impl<'a> Invocation<'a> {
    /// The invocation with `--project` set to `project`, on `ports`.
    pub fn new(project: Option<String>, ports: &'a dyn CliPorts) -> Self {
        let service = ports.service();
        Self {
            project,
            ports,
            notifier: RunningServiceNotifier::new(service.clone()),
            service,
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
        &self.notifier
    }

    /// muster's local state.
    ///
    /// # Errors
    /// Returns the [`CliError::Config`] of a database that cannot be opened.
    pub fn store(&self) -> Result<Arc<dyn LocalState>, CliError> {
        Ok(self.ports.open_state()?)
    }

    /// The use case an edit goes through: writes to `store`, then tells a
    /// running service.
    pub fn editor<'s>(&'s self, store: &'s dyn LocalState) -> StackEditor<'s> {
        StackEditor::builder()
            .stacks(store)
            .settings(store)
            .edits(store)
            .notifier(&self.notifier)
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

    /// The registered project the command acts on.
    ///
    /// # Errors
    /// Returns [`CliError::NotAProject`] outside every project, or the
    /// [`CliError`] of a project that cannot be resolved.
    pub fn project(&self, store: &dyn LocalState) -> Result<Project, CliError> {
        Ok(self.target(store)?.registered()?)
    }

    /// The project a command that edits a stack acts on, as [`Self::target`]
    /// finds it, after a note on stderr when `MUSTER_PROJECT` chose it over
    /// another project the current folder is in.
    ///
    /// # Errors
    /// Returns the [`CliError`] of a project that cannot be resolved.
    pub fn editing_target(&self, store: &dyn LocalState) -> Result<ProjectTarget, CliError> {
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
    pub fn editing_project(&self, store: &dyn LocalState) -> Result<Project, CliError> {
        Ok(self.editing_target(store)?.registered()?)
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

#[cfg(test)]
mod tests {
    use std::{fs, sync::Mutex};

    use tempfile::TempDir;

    use super::*;
    use crate::{
        adapter::{
            cli::mcp_launch::McpLaunch,
            client::{ClientError, ServiceStatus},
            no_state_dir::NoStateDir,
            service::{ProjectStatus, ServiceRequest},
            store::{
                SqliteStore,
                test_support::{fresh_store, spec},
            },
        },
        domain::{
            config::{ConfigError, ImportMode},
            port::StackReader,
            value::ProjectRoot,
        },
        error::MusterError,
    };

    /// A service that is never reached, recording what it was told.
    #[derive(Default)]
    struct RecordingService {
        told: Mutex<Vec<ServiceRequest>>,
    }

    impl ServiceAccess for RecordingService {
        fn connect(&self) -> Result<ServiceClient, ClientError> {
            Err(ClientError::NoStateDir(NoStateDir))
        }

        fn loaded_status(
            &self,
            _project: &ProjectRoot,
        ) -> Result<Option<ProjectStatus>, ClientError> {
            Ok(None)
        }

        fn status(&self) -> Result<ServiceStatus, ClientError> {
            Err(ClientError::NoStateDir(NoStateDir))
        }

        fn stop(&self) -> Result<bool, ClientError> {
            Ok(false)
        }

        fn notify(&self, request: &ServiceRequest) {
            self.told.lock().unwrap().push(request.clone());
        }
    }

    /// Ports over a scratch database and a [`RecordingService`].
    struct FakePorts {
        store: SqliteStore,
        service: Arc<RecordingService>,
    }

    impl CliPorts for FakePorts {
        fn open_state(&self) -> Result<Arc<dyn LocalState>, ConfigError> {
            Ok(Arc::new(self.store.clone()))
        }

        fn existing_state(&self) -> Result<Option<Arc<dyn LocalState>>, ConfigError> {
            Ok(Some(Arc::new(self.store.clone())))
        }

        fn service(&self) -> Arc<dyn ServiceAccess> {
            self.service.clone()
        }

        fn serve_mcp(&self, _launch: McpLaunch) -> Result<(), MusterError> {
            Ok(())
        }

        fn mcp_config(&self, _executable: &Path) -> String {
            String::new()
        }

        fn run_service(&self) -> Result<(), MusterError> {
            Ok(())
        }

        fn run_holder(&self, _spec: &Path) -> Result<(), MusterError> {
            Ok(())
        }
    }

    fn ports() -> (FakePorts, TempDir) {
        let (store, scratch) = fresh_store();
        let ports = FakePorts {
            store,
            service: Arc::new(RecordingService::default()),
        };
        (ports, scratch)
    }

    /// An edit goes to the injected state and is told to the injected
    /// service, and reaching the service goes through the injected port, so
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

        assert!(ports.store.stack(&root).unwrap().is_some());
        assert_eq!(*ports.service.told.lock().unwrap(), vec![
            ServiceRequest::StackChanged {
                project: root.clone()
            }
        ]);
        assert!(matches!(
            invocation.connect(),
            Err(CliError::Client(ClientError::NoStateDir(NoStateDir)))
        ));
        assert!(!invocation.service().stop().unwrap());
    }
}
