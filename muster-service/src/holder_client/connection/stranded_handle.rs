use std::{
    path::PathBuf,
    time::{Duration, SystemTime},
};

use muster_domain::{
    port::{OutputSink, ProcessHandle, ProcessInput},
    process::StopSignal,
    pty::{ExitOutcome, ProcessOutput, PtyError, PtySize, ScreenSource},
    value::{OutputCursor, Pid},
};
use muster_protocol::holder::HolderRecord;

/// Why a stranded process refuses what only a running one can take.
const ALREADY_EXITED: &str = "the process already exited while its holder could not be reached";

/// The handle of a process whose holder gave up waiting to have its exit
/// collected and left it beside its record: it reports that exit to its
/// sink at once, as a holder hands a waiting exit to a new supervisor, so
/// the exit is recorded and its restart policy applies like any other.
/// Acknowledging it, or letting the process go, removes the record and the
/// exit; dropping it leaves both for the next service. It never signals the
/// child's pid, which another process may hold by now.
pub(crate) struct StrandedHandle {
    /// The record the holder left.
    record: HolderRecord,
    /// The directory of holder records it lies in.
    records: PathBuf,
}

impl StrandedHandle {
    /// Hands `outcome`, the exit the holder `record` describes left in
    /// `records`, to `sink`, and returns the handle of its process.
    pub(crate) fn deliver(
        record: HolderRecord,
        records: PathBuf,
        outcome: ExitOutcome,
        sink: &dyn OutputSink,
    ) -> Self {
        sink.send(ProcessOutput::Exited(outcome));
        Self { record, records }
    }

    /// Removes the record and the exit beside it.
    ///
    /// # Errors
    /// Returns the I/O error from removing them.
    fn forget(&self) -> Result<(), PtyError> {
        Ok(self.record.remove(&self.records)?)
    }

    /// The refusal of what only a running process can take.
    fn exited() -> PtyError {
        PtyError::Unsupported(ALREADY_EXITED.to_string())
    }
}

impl ProcessHandle for StrandedHandle {
    fn process_id(&self) -> Option<Pid> {
        Some(self.record.child_pid())
    }

    fn input(&self) -> Option<Box<dyn ProcessInput>> {
        None
    }

    fn screen_source(&self) -> ScreenSource {
        ScreenSource::Kept
    }

    fn abandon(self: Box<Self>) {
        let _ = self.forget();
    }

    fn take_initial_screen(&mut self) -> Option<Vec<u8>> {
        None
    }

    fn screen_size(&self) -> Option<PtySize> {
        None
    }

    fn attached_clients(&self) -> u32 {
        0
    }

    fn started_at(&self) -> Option<SystemTime> {
        Some(self.record.child_started_at())
    }

    fn run_start(&self) -> Option<OutputCursor> {
        None
    }

    fn output_end(&self) -> Option<OutputCursor> {
        None
    }

    fn take_backlog(&mut self) -> Option<(OutputCursor, Vec<u8>)> {
        None
    }

    fn acknowledge_exit(&mut self) -> Result<(), PtyError> {
        self.forget()
    }

    fn write_input(&mut self, _bytes: &[u8]) -> Result<(), PtyError> {
        Err(Self::exited())
    }

    fn resize(&mut self, _size: PtySize) -> Result<(), PtyError> {
        Err(Self::exited())
    }

    fn pause(&mut self) -> Result<(), PtyError> {
        Err(Self::exited())
    }

    fn resume(&mut self) -> Result<(), PtyError> {
        Err(Self::exited())
    }

    /// The process already exited, and its exit is on its way to the sink.
    fn terminate(&mut self, _signal: StopSignal, _grace: Duration) -> Result<(), PtyError> {
        Ok(())
    }

    /// The process already exited, and its exit is on its way to the sink.
    fn kill(&mut self) -> Result<(), PtyError> {
        Ok(())
    }
}
