use std::{thread, time::Duration};

use crossbeam_channel::{Receiver, Sender, bounded};

use super::{
    config::HolderConfig, holder_launcher::HolderLauncher, holders::Holders, server::HolderServer,
};
use crate::{adapter::pty::PortablePtyRunner, domain::pty::PtyError};

/// Runs each holder on a thread of the test process over a real PTY, once its
/// start opens, is dropped, or `wait` passes.
pub(crate) struct ThreadLauncher {
    holders: Holders,
    start: Receiver<()>,
    /// Keeps `start` shut for a launcher that only delays.
    _held: Option<Sender<()>>,
    wait: Duration,
}

impl ThreadLauncher {
    /// A launcher starting each holder at once into `holders`.
    pub(crate) fn new(holders: Holders) -> Self {
        Self::delayed(holders, Duration::ZERO)
    }

    /// A launcher starting each holder `delay` after its launch, as a holder
    /// slow to start does.
    pub(crate) fn delayed(holders: Holders, delay: Duration) -> Self {
        let (held, start) = bounded(0);
        Self {
            holders,
            start,
            _held: Some(held),
            wait: delay,
        }
    }

    /// A launcher starting each holder once `gate` opens or is dropped, or at
    /// the latest after `wait`.
    pub(crate) fn gated(holders: Holders, gate: Receiver<()>, wait: Duration) -> Self {
        Self {
            holders,
            start: gate,
            _held: None,
            wait,
        }
    }
}

impl HolderLauncher for ThreadLauncher {
    fn launch(&self, config: HolderConfig) -> Result<(), PtyError> {
        let start = self.start.clone();
        let wait = self.wait;
        let holder = thread::spawn(move || {
            let _ = start.recv_timeout(wait);
            HolderServer::run(config, &PortablePtyRunner)
        });
        self.holders.lock().unwrap().push(holder);
        Ok(())
    }
}
