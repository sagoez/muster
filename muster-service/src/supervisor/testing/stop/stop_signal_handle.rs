use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use getset::Getters;
use muster_domain::{process::StopSignal, pty::PtyError};
use typed_builder::TypedBuilder;

use super::stop_signals::StopSignals;
use crate::supervisor::testing::StubHandle;

/// A handle that distinguishes graceful termination from a hard kill.
#[derive(Getters, TypedBuilder)]
#[getset(get = "pub(crate)")]
pub(crate) struct StopSignalHandle {
    signals: Arc<StopSignals>,
}

impl StubHandle for StopSignalHandle {
    fn terminate(&mut self, signal: StopSignal, grace: Duration) -> Result<(), PtyError> {
        self.signals.terminates().fetch_add(1, Ordering::SeqCst);
        *self.signals.termination().lock().unwrap() = Some((signal, grace));
        Ok(())
    }

    fn kill(&mut self) -> Result<(), PtyError> {
        self.signals.kills().fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
