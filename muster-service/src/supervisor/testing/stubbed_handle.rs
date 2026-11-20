use std::time::{Duration, SystemTime};

use muster_domain::{
    port::{ProcessHandle, ProcessInput},
    process::{LogPattern, StopSignal},
    pty::{PtyError, PtySize, ScreenSource},
    value::{OutputCursor, Pid},
};

use super::stub_handle::StubHandle;

/// Runs a [`StubHandle`] as a [`ProcessHandle`], abandoning it with a kill.
pub(crate) struct StubbedHandle<S>(S);

impl<S: StubHandle> StubbedHandle<S> {
    /// Wraps `stub`.
    pub(crate) fn new(stub: S) -> Self {
        Self(stub)
    }
}

impl<S: StubHandle> ProcessHandle for StubbedHandle<S> {
    fn process_id(&self) -> Option<Pid> {
        self.0.process_id()
    }

    fn input(&self) -> Option<Box<dyn ProcessInput>> {
        self.0.input()
    }

    fn screen_source(&self) -> ScreenSource {
        self.0.screen_source()
    }

    fn abandon(self: Box<Self>) {
        let mut stub = self.0;
        let _ = stub.kill();
    }

    fn take_initial_screen(&mut self) -> Option<Vec<u8>> {
        self.0.take_initial_screen()
    }

    fn screen_size(&self) -> Option<PtySize> {
        self.0.screen_size()
    }

    fn attached_clients(&self) -> u32 {
        self.0.attached_clients()
    }

    fn started_at(&self) -> Option<SystemTime> {
        self.0.started_at()
    }

    fn run_start(&self) -> Option<OutputCursor> {
        self.0.run_start()
    }

    fn output_end(&self) -> Option<OutputCursor> {
        self.0.output_end()
    }

    fn take_backlog(&mut self) -> Option<(OutputCursor, Vec<u8>)> {
        self.0.take_backlog()
    }

    fn replay_matching(&mut self, patterns: &[LogPattern]) -> Option<OutputCursor> {
        self.0.replay_matching(patterns)
    }

    fn acknowledge_exit(&mut self) -> Result<(), PtyError> {
        self.0.acknowledge_exit()
    }

    fn write_input(&mut self, bytes: &[u8]) -> Result<(), PtyError> {
        self.0.write_input(bytes)
    }

    fn resize(&mut self, size: PtySize) -> Result<(), PtyError> {
        self.0.resize(size)
    }

    fn pause(&mut self) -> Result<(), PtyError> {
        self.0.pause()
    }

    fn resume(&mut self) -> Result<(), PtyError> {
        self.0.resume()
    }

    fn terminate(&mut self, signal: StopSignal, grace: Duration) -> Result<(), PtyError> {
        self.0.terminate(signal, grace)
    }

    fn kill(&mut self) -> Result<(), PtyError> {
        self.0.kill()
    }
}
