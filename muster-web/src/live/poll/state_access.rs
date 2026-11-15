use std::{path::PathBuf, sync::Arc};

use muster_domain::{config::ConfigError, port::LocalState};

use super::file_identity::FileIdentity;
use crate::{
    floor::{ProjectsReading, StateTrouble},
    opened_state::OpenedState,
    view_deps::StateOpener,
};

/// muster's state, opened read-only once it can be and kept open, opened
/// again after a read of it failed or once its file is another one.
pub struct StateAccess {
    /// What opens it.
    opener: StateOpener,
    /// The file it lives in.
    database: PathBuf,
    /// It, while open, with the file it was opened from.
    open: Option<(Arc<dyn LocalState>, FileIdentity)>,
}

impl StateAccess {
    /// The state `opener` opens from the file `database`, not opened yet.
    pub fn new(opener: StateOpener, database: PathBuf) -> Self {
        Self {
            opener,
            database,
            open: None,
        }
    }

    /// The state, opened now unless it is open already from the file that
    /// is there now: one deleted and made again is a new database, which
    /// the connection kept open would never see.
    ///
    /// # Errors
    /// Returns what reading the projects finds instead: no database yet,
    /// one an older or a newer muster wrote, or one that does not open.
    pub fn store(&mut self) -> Result<Arc<dyn LocalState>, ProjectsReading> {
        let identity = FileIdentity::of(&self.database);
        if let Some((store, opened)) = &self.open
            && identity == Some(*opened)
        {
            return Ok(Arc::clone(store));
        }
        self.open = None;
        let trouble = |health| Err(ProjectsReading::Trouble(health));
        match (self.opener)() {
            Ok(OpenedState::Current(store)) => {
                if let Some(identity) = identity {
                    self.open = Some((Arc::clone(&store), identity));
                }
                Ok(store)
            },
            Ok(OpenedState::Absent) => trouble(StateTrouble::NoState),
            Ok(OpenedState::Older) => trouble(StateTrouble::OlderSchema),
            Err(ConfigError::UnsupportedSchema { .. }) => trouble(StateTrouble::NewerSchema),
            Err(_) => trouble(StateTrouble::Unreadable),
        }
    }

    /// Closes the state after a read of it failed, so the next read opens
    /// it afresh and finds how it stands now.
    pub fn forget(&mut self) {
        self.open = None;
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use muster_os::test_scratch::short_scratch;
    use muster_store::SqliteStore;

    use super::*;

    /// The state over the file at `database`, each open a store at `store`,
    /// and how many times it was opened. The store lives apart from the
    /// watched file, so replacing that file never touches a live database.
    fn counted(database: PathBuf, store: PathBuf) -> (StateAccess, Arc<AtomicUsize>) {
        let opens = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&opens);
        let opener: StateOpener = Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
            let store: Arc<dyn LocalState> = Arc::new(SqliteStore::open(&store)?);
            Ok(OpenedState::Current(store))
        });
        (StateAccess::new(opener, database), opens)
    }

    /// The open state is kept while its file stays, and opened afresh once
    /// another file takes its place.
    #[test]
    fn a_database_made_again_is_opened_afresh() {
        let scratch = short_scratch("mv-");
        let database = scratch.path().join("muster.db");
        fs::write(&database, b"first").unwrap();
        let (mut state, opens) = counted(database.clone(), scratch.path().join("store.db"));

        state.store().unwrap();
        state.store().unwrap();
        assert_eq!(opens.load(Ordering::SeqCst), 1);

        let replacement = scratch.path().join("new.db");
        fs::write(&replacement, b"second").unwrap();
        fs::rename(&replacement, &database).unwrap();
        state.store().unwrap();

        assert_eq!(opens.load(Ordering::SeqCst), 2);
    }

    /// A database an older muster wrote reads as an older schema.
    #[test]
    fn an_older_database_reads_as_an_older_schema() {
        let mut state = StateAccess::new(
            Arc::new(|| Ok(OpenedState::Older)),
            PathBuf::from("/nowhere/muster.db"),
        );

        assert_eq!(
            state.store().err(),
            Some(ProjectsReading::Trouble(StateTrouble::OlderSchema))
        );
    }
}
