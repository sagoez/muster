use std::thread;

use crossbeam_channel::Receiver;

use super::{holders::Holders, runtime_testing::WAIT};
use crate::{
    adapter::holder::{HolderConfig, HolderLauncher, HolderServer},
    domain::pty::PtyError,
};

/// Runs each holder on a thread of the test process once `gate` opens or
/// is dropped, as a holder that is slow to start does.
pub(super) struct GatedLauncher {
    pub(super) holders: Holders,
    pub(super) gate: Receiver<()>,
}

impl HolderLauncher for GatedLauncher {
    fn launch(&self, config: HolderConfig) -> Result<(), PtyError> {
        let gate = self.gate.clone();
        let holder = thread::spawn(move || {
            let _ = gate.recv_timeout(WAIT);
            HolderServer::run(config)
        });
        self.holders.lock().unwrap().push(holder);
        Ok(())
    }
}
