use std::{path::Path, sync::Arc};

use super::{cli_ports::CliPorts, mcp_launch::McpLaunch};
use crate::{
    adapter::{
        client::{ServiceAccess, fake_service::FakeService},
        store::SqliteStore,
    },
    domain::{config::ConfigError, port::LocalState},
    error::MusterError,
};

/// Ports over a scratch database and a [`FakeService`], serving nothing.
pub(super) struct FakePorts {
    store: SqliteStore,
    service: Arc<FakeService>,
}

impl FakePorts {
    /// Ports over `store` and a fresh [`FakeService`].
    pub(super) fn new(store: SqliteStore) -> Self {
        Self {
            store,
            service: Arc::new(FakeService::default()),
        }
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
