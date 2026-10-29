use std::time::{Duration, SystemTime};

use crossbeam_channel::Sender;
use portable_pty::{ChildKiller, MasterPty};
use rustix::process::Signal;

use super::{
    pty_input::PtyInput,
    runner::{
        signal_group, signallable, system_error, terminate_group, to_portable_size, unix_signal,
    },
};
use crate::domain::{
    port::{ProcessHandle, ProcessInput},
    process::StopSignal,
    pty::{PtyError, PtySize, ScreenSource},
    value::{OutputCursor, Pid},
};

/// Live handle to a PTY-backed process.
pub(super) struct PtyProcessHandle {
    pub(super) master: Box<dyn MasterPty + Send>,
    pub(super) input: PtyInput,
    pub(super) killer: Box<dyn ChildKiller + Send + Sync>,
    pub(super) grace_tx: Sender<Duration>,
    pub(super) pid: Option<Pid>,
}

impl ProcessHandle for PtyProcessHandle {
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
        None
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
        Ok(())
    }

    fn process_id(&self) -> Option<Pid> {
        self.pid
    }

    fn input(&self) -> Option<Box<dyn ProcessInput>> {
        Some(Box::new(self.input.clone()))
    }

    /// A bare PTY has nothing but its reader's rendering of the output.
    fn screen_source(&self) -> ScreenSource {
        ScreenSource::Kept
    }

    /// Kills the process group; nothing else holds the process.
    fn abandon(mut self: Box<Self>) {
        let _ = self.kill();
    }

    fn write_input(&mut self, bytes: &[u8]) -> Result<(), PtyError> {
        self.input.write_input(bytes)
    }

    fn resize(&mut self, size: PtySize) -> Result<(), PtyError> {
        self.master
            .resize(to_portable_size(size))
            .map_err(system_error)
    }

    fn pause(&mut self) -> Result<(), PtyError> {
        signal_group(self.group()?, Signal::STOP)
    }

    fn resume(&mut self) -> Result<(), PtyError> {
        signal_group(self.group()?, Signal::CONT)
    }

    fn terminate(&mut self, signal: StopSignal, grace: Duration) -> Result<(), PtyError> {
        // Publish the grace before signalling: if the signal makes the direct shell
        // exit immediately, its waiter still observes the intended deadline.
        let _ = self.grace_tx.try_send(grace);
        // A paused command cannot handle a shutdown signal until it resumes.
        let group = self.group()?;
        let _ = signal_group(group, Signal::CONT);
        signal_group(group, unix_signal(signal))
    }

    fn kill(&mut self) -> Result<(), PtyError> {
        terminate_group(self.pid, &mut self.killer)
    }
}

impl PtyProcessHandle {
    /// The process group the child leads, which every signal reaches.
    ///
    /// # Errors
    /// Returns [`PtyError::Unsupported`] when the platform gave no pid.
    fn group(&self) -> Result<Pid, PtyError> {
        signallable(self.pid)
    }
}
