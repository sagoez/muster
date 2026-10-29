use std::thread;

use super::holders::Holders;
use crate::{
    adapter::holder::{HolderConfig, HolderLauncher, HolderServer},
    domain::pty::PtyError,
};

/// Runs each holder on a thread of the test process.
pub(super) struct ThreadLauncher(pub(super) Holders);

impl HolderLauncher for ThreadLauncher {
    fn launch(&self, config: HolderConfig) -> Result<(), PtyError> {
        let holder = thread::spawn(move || HolderServer::run(config));
        self.0.lock().unwrap().push(holder);
        Ok(())
    }
}
