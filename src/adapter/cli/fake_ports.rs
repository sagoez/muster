use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use rustix::process::geteuid;
use tempfile::TempDir;

use super::{cli_ports::CliPorts, mcp_launch::McpLaunch};
use crate::{
    adapter::{
        client::{
            ServiceAccess, fake_service::FakeService, recording_notifier::RecordingNotifier,
            recording_workspace::RecordingWorkspace,
        },
        no_state_dir::NoStateDir,
        private_dir::claimed_by,
        shell::ShellFiles,
        store::SqliteStore,
    },
    domain::{
        config::ConfigError,
        port::{LocalState, ShellRegistry, StateChangeNotifier, TerminalWorkspace},
    },
    error::MusterError,
};

/// The state directory's name under the scratch folder standing for its
/// parent.
const STATE_DIR: &str = "muster";

/// Ports over a scratch database, a [`FakeService`], a
/// [`RecordingNotifier`] and shell files in a scratch state directory,
/// serving nothing, run as this process's user or another, outside a
/// terminal workspace or in a [`RecordingWorkspace`].
pub(super) struct FakePorts {
    store: SqliteStore,
    service: Arc<FakeService>,
    notifier: Arc<RecordingNotifier>,
    /// The scratch folder standing for the state directory's parent.
    state_home: TempDir,
    /// The effective user the ports run as.
    user: u32,
    /// The terminal workspace the ports run in, if any.
    workspace: Option<Arc<RecordingWorkspace>>,
}

impl FakePorts {
    /// Ports over `store`, a fresh [`FakeService`], a fresh notifier and a
    /// fresh scratch state directory.
    pub(super) fn new(store: SqliteStore) -> Self {
        Self::with_service(store, FakeService::default())
    }

    /// Ports over `store` and `service`, a fresh notifier and a fresh
    /// scratch state directory.
    pub(super) fn with_service(store: SqliteStore, service: FakeService) -> Self {
        let state_home = TempDir::new().unwrap();
        fs::create_dir(state_home.path().join(STATE_DIR)).unwrap();
        Self {
            store,
            service: Arc::new(service),
            notifier: Arc::new(RecordingNotifier::default()),
            state_home,
            user: geteuid().as_raw(),
            workspace: None,
        }
    }

    /// These ports run as the user `uid`, as a root shell that kept this
    /// user's home would.
    pub(super) fn running_as(self, uid: u32) -> Self {
        Self { user: uid, ..self }
    }

    /// These ports with no state directory yet, as on a fresh install.
    pub(super) fn fresh_install(self) -> Self {
        fs::remove_dir_all(self.state_dir()).unwrap();
        self
    }

    /// The state directory, which may not exist yet.
    pub(super) fn state_dir(&self) -> PathBuf {
        self.state_home.path().join(STATE_DIR)
    }

    /// These ports run inside `workspace`.
    pub(super) fn in_workspace(self, workspace: RecordingWorkspace) -> Self {
        Self {
            workspace: Some(Arc::new(workspace)),
            ..self
        }
    }

    /// The workspace these ports run in, to read what it was asked.
    pub(super) fn workspace(&self) -> Option<&RecordingWorkspace> {
        self.workspace.as_deref()
    }

    /// The notifier these ports hand out, to read what it was told.
    pub(super) fn notifier(&self) -> &RecordingNotifier {
        &self.notifier
    }

    /// The database every command of these ports reads and writes.
    pub(super) fn store(&self) -> &SqliteStore {
        &self.store
    }

    /// The service these ports hand out, to read what it was told.
    pub(super) fn fake_service(&self) -> &FakeService {
        &self.service
    }
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

    fn change_notifier(&self) -> Arc<dyn StateChangeNotifier> {
        self.notifier.clone()
    }

    fn shell_registry(&self) -> Result<Arc<dyn ShellRegistry>, NoStateDir> {
        Ok(Arc::new(ShellFiles::under(&self.state_dir())))
    }

    fn claim_state_dir(&self) -> bool {
        claimed_by(&self.state_dir(), self.user)
    }

    fn terminal_workspace(&self) -> Option<Arc<dyn TerminalWorkspace>> {
        self.workspace
            .clone()
            .map(|workspace| workspace as Arc<dyn TerminalWorkspace>)
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::{
        adapter::store::test_support::fresh_store,
        domain::terminal_workspace::{PaneCommand, PaneFocus, PaneRequest},
    };

    /// Ports outside a workspace hand the CLI none; ports in one hand it the
    /// workspace whose asks the test then reads.
    #[test]
    fn ports_hand_the_cli_the_workspace_they_run_in() {
        let (store, _scratch) = fresh_store();
        let outside = FakePorts::new(store.clone());
        let inside = FakePorts::new(store).in_workspace(RecordingWorkspace::opening("w1:p4"));
        let request = PaneRequest::builder()
            .cwd(PathBuf::from("/srv/shop"))
            .command(
                PaneCommand::builder()
                    .program(PathBuf::from("muster"))
                    .args(Vec::new())
                    .build(),
            )
            .focus(PaneFocus::Take)
            .direction(None)
            .build();

        assert!(outside.terminal_workspace().is_none());
        assert!(outside.workspace().is_none());
        let opened = inside
            .terminal_workspace()
            .unwrap()
            .open_beside(&request)
            .unwrap();
        assert_eq!(opened.pane().as_ref(), "w1:p4");
        assert_eq!(inside.workspace().unwrap().requests(), [request]);
    }
}
