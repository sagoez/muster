use std::{path::Path, sync::Arc};

use rustix::process::geteuid;
use tempfile::TempDir;

use super::{cli_ports::CliPorts, mcp_launch::McpLaunch};
use crate::{
    adapter::{
        client::{ServiceAccess, fake_service::FakeService, recording_notifier::RecordingNotifier},
        no_state_dir::NoStateDir,
        private_dir::owned_by,
        shell::ShellFiles,
        store::SqliteStore,
    },
    domain::{
        config::ConfigError,
        port::{LocalState, ShellRegistry, StateChangeNotifier},
    },
    error::MusterError,
};

/// Ports over a scratch database, a [`FakeService`], a
/// [`RecordingNotifier`] and shell files in a scratch state directory,
/// serving nothing, run as this process's user or another.
pub(super) struct FakePorts {
    store: SqliteStore,
    service: Arc<FakeService>,
    notifier: Arc<RecordingNotifier>,
    state_dir: TempDir,
    /// The effective user the ports run as.
    user: u32,
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
        Self {
            store,
            service: Arc::new(service),
            notifier: Arc::new(RecordingNotifier::default()),
            state_dir: TempDir::new().unwrap(),
            user: geteuid().as_raw(),
        }
    }

    /// These ports run as the user `uid`, as a root shell that kept this
    /// user's home would.
    pub(super) fn running_as(self, uid: u32) -> Self {
        Self { user: uid, ..self }
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
        Ok(Arc::new(ShellFiles::under(self.state_dir.path())))
    }

    fn owns_state_dir(&self) -> bool {
        owned_by(self.state_dir.path(), self.user)
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
