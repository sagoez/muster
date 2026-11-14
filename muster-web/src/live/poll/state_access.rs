use std::sync::Arc;

use muster_domain::{config::ConfigError, port::LocalState};

use crate::{floor::ProjectsReading, view_deps::StateOpener};

/// muster's state, opened read-only once it can be and kept open, opened
/// again after a read of it failed.
pub struct StateAccess {
    /// What opens it.
    opener: StateOpener,
    /// It, while open.
    store: Option<Arc<dyn LocalState>>,
}

impl StateAccess {
    /// The state `opener` opens, not opened yet.
    pub fn new(opener: StateOpener) -> Self {
        Self {
            opener,
            store: None,
        }
    }

    /// The state, opened now unless it is open already.
    ///
    /// # Errors
    /// Returns what reading the projects finds instead: no database yet,
    /// one a newer muster wrote, or one that does not open.
    pub fn store(&mut self) -> Result<Arc<dyn LocalState>, ProjectsReading> {
        if let Some(store) = &self.store {
            return Ok(Arc::clone(store));
        }
        match (self.opener)() {
            Ok(Some(store)) => {
                self.store = Some(Arc::clone(&store));
                Ok(store)
            },
            Ok(None) => Err(ProjectsReading::NoState),
            Err(ConfigError::UnsupportedSchema { .. }) => Err(ProjectsReading::NewerSchema),
            Err(_) => Err(ProjectsReading::Unreadable),
        }
    }

    /// Closes the state after a read of it failed, so the next read opens
    /// it afresh and finds how it stands now.
    pub fn forget(&mut self) {
        self.store = None;
    }
}
