use std::time::Duration;

use crate::domain::{
    process::StopSignal,
    pty::{ProcessOutput, PtyError, PtySize, SpawnRequest},
};

/// Sink for a single process's output events, supplied by the runtime. The
/// concrete implementation (e.g. a channel into the event loop) lives in an
/// adapter, keeping the domain free of any transport type.
pub trait OutputSink: Send + Sync + 'static {
    /// Delivers one output event from the process to the runtime.
    fn send(&self, output: ProcessOutput);
}

/// Handle to a spawned process: write input, resize, or terminate it.
pub trait ProcessHandle: Send {
    /// Writes raw input bytes to the process's PTY.
    ///
    /// # Errors
    /// Returns a `PtyError` if the write fails.
    fn write_input(&mut self, bytes: &[u8]) -> Result<(), PtyError>;

    /// Resizes the process's PTY.
    ///
    /// # Errors
    /// Returns a `PtyError` if the resize fails.
    fn resize(&mut self, size: PtySize) -> Result<(), PtyError>;

    /// Suspends the process (SIGSTOP-equivalent), leaving it alive.
    ///
    /// # Errors
    /// Returns a `PtyError` if the signal cannot be sent.
    fn pause(&mut self) -> Result<(), PtyError>;

    /// Resumes a previously suspended process (SIGCONT-equivalent).
    ///
    /// # Errors
    /// Returns a `PtyError` if the signal cannot be sent.
    fn resume(&mut self) -> Result<(), PtyError>;

    /// Requests graceful process termination with `signal` and `grace`, falling
    /// back to [`Self::kill`] for adapters without a distinct mechanism.
    ///
    /// # Errors
    /// Returns a `PtyError` if the termination signal cannot be sent.
    fn terminate(&mut self, _signal: StopSignal, _grace: Duration) -> Result<(), PtyError> {
        self.kill()
    }

    /// Forcibly terminates the process.
    ///
    /// # Errors
    /// Returns a `PtyError` if the kill signal cannot be sent.
    fn kill(&mut self) -> Result<(), PtyError>;
}

/// Driven port: spawns a process under a PTY and streams its output to a sink.
pub trait ProcessRunner {
    /// Spawns `request`'s command, delivering output to `sink`, and returns a
    /// handle for interacting with the running process.
    ///
    /// # Errors
    /// Returns a `PtyError` if the process cannot be spawned.
    fn spawn(
        &self,
        request: SpawnRequest,
        sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError>;
}
