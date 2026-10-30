use std::{thread, time::Duration};

use muster::{
    adapter::holder::{ExecHolderLauncher, HolderConfig, HolderLauncher},
    domain::pty::PtyError,
};

/// How long a launch is held, so a replacement waits on it.
pub const GATE_HOLD: Duration = Duration::from_millis(1500);

/// Launches real holders from the built binary, but only after holding each
/// launch for [`GATE_HOLD`], so a launch is in flight when the service is
/// asked to make way.
pub struct HeldLauncher(ExecHolderLauncher);

impl HeldLauncher {
    /// Holds each launch of `exec`.
    pub fn new(exec: ExecHolderLauncher) -> Self {
        Self(exec)
    }
}

impl HolderLauncher for HeldLauncher {
    /// Launches `config` once [`GATE_HOLD`] has passed.
    fn launch(&self, config: HolderConfig) -> Result<(), PtyError> {
        thread::sleep(GATE_HOLD);
        self.0.launch(config)
    }
}
