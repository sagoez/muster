use std::{fs, thread};

use super::{
    config::HolderConfig, holder_launcher::HolderLauncher, holders::Holders, launcher::run_holder,
    sabotage::Sabotage,
};
use crate::{adapter::pty::PortablePtyRunner, domain::pty::PtyError};

/// A spec no holder can parse.
pub(super) const CORRUPT_SPEC: &[u8] = b"{not a spec";

/// Hands each holder its spec file and runs it through the real entry point
/// on a thread, sabotaged first.
pub(super) struct SpecLauncher {
    holders: Holders,
    sabotage: Option<Sabotage>,
}

impl SpecLauncher {
    /// A launcher pushing its holder threads to `holders`, sabotaged as given.
    pub(super) fn new(holders: Holders, sabotage: Option<Sabotage>) -> Self {
        Self { holders, sabotage }
    }
}

impl HolderLauncher for SpecLauncher {
    fn launch(&self, config: HolderConfig) -> Result<(), PtyError> {
        let spec = config.spec_path();
        match &self.sabotage {
            Some(Sabotage::CorruptSpec) => fs::write(&spec, CORRUPT_SPEC).unwrap(),
            Some(Sabotage::RemoveDir(dir)) => {
                config.write_private(&spec).unwrap();
                fs::remove_dir(dir).unwrap();
            },
            None => config.write_private(&spec).unwrap(),
        }
        let holder = thread::spawn(move || run_holder(&spec, &PortablePtyRunner));
        self.holders.lock().unwrap().push(holder);
        Ok(())
    }
}
