use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use muster_client::OpenedState;
use muster_domain::{config::ConfigError, port::LocalState};

use super::state_access::StateAccess;

/// The state over the file `database`, each open handing out the store
/// `open` gives, and how many times it was opened.
pub(crate) fn counted(
    database: PathBuf,
    open: impl Fn() -> Result<Arc<dyn LocalState>, ConfigError> + Send + Sync + 'static,
) -> (StateAccess, Arc<AtomicUsize>) {
    let opens = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&opens);
    let state = StateAccess::new(
        Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
            open().map(OpenedState::Current)
        }),
        database,
    );
    (state, opens)
}
