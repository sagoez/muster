use std::{
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use super::{retry_stop_signals::RetryStopSignals, terminate_outcome::TerminateOutcome};
use crate::domain::{
    port::{ProcessHandle, ProcessInput},
    process::StopSignal,
    pty::{PtyError, PtySize, ScreenSource},
    value::Pid,
};

/// A handle whose graceful termination follows its configured outcome and
/// whose hard kill succeeds on the second attempt.
pub(crate) struct RetryStopHandle {
    pub(crate) signals: Arc<RetryStopSignals>,
    pub(crate) terminate_outcome: TerminateOutcome,
}

impl ProcessHandle for RetryStopHandle {
    fn process_id(&self) -> Option<Pid> {
        None
    }

    fn input(&self) -> Option<Box<dyn ProcessInput>> {
        None
    }

    fn screen_source(&self) -> ScreenSource {
        ScreenSource::Kept
    }

    fn abandon(mut self: Box<Self>) {
        let _ = self.kill();
    }

    fn write_input(&mut self, _bytes: &[u8]) -> Result<(), PtyError> {
        Ok(())
    }

    fn resize(&mut self, _size: PtySize) -> Result<(), PtyError> {
        Ok(())
    }

    fn pause(&mut self) -> Result<(), PtyError> {
        Ok(())
    }

    fn resume(&mut self) -> Result<(), PtyError> {
        Ok(())
    }

    fn terminate(&mut self, _signal: StopSignal, _grace: Duration) -> Result<(), PtyError> {
        self.signals.terminates.fetch_add(1, Ordering::SeqCst);
        match self.terminate_outcome {
            TerminateOutcome::Succeeds => Ok(()),
            TerminateOutcome::Fails => Err(PtyError::Unsupported("signal failed".to_string())),
        }
    }

    fn kill(&mut self) -> Result<(), PtyError> {
        let attempt = self.signals.kills.fetch_add(1, Ordering::SeqCst);
        if attempt == 0 {
            Err(PtyError::Unsupported("signal failed".to_string()))
        } else {
            Ok(())
        }
    }
}
