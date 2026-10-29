use std::{
    sync::Arc,
    time::{Duration, SystemTime},
};

use crate::domain::{
    process::StopSignal,
    pty::{ProcessOutput, PtyError, PtySize, ScreenSource, SpawnRequest},
    value::OutputCursor,
};

/// Sink for a single process's output events, supplied by the runtime. The
/// concrete implementation (e.g. a channel into the event loop) lives in an
/// adapter, keeping the domain free of any transport type.
pub trait OutputSink: Send + Sync + 'static {
    /// Delivers one output event from the process to the runtime.
    fn send(&self, output: ProcessOutput);
}

/// The input half of a process's terminal, writable from a thread other than
/// the one controlling the process.
pub trait ProcessInput: Send {
    /// Writes raw input bytes to the process's PTY, blocking while it is full.
    ///
    /// # Errors
    /// Returns a `PtyError` if the write fails.
    fn write_input(&mut self, bytes: &[u8]) -> Result<(), PtyError>;
}

/// Handle to a spawned process: write input, resize, or terminate it.
pub trait ProcessHandle: Send {
    /// The OS process ID of the spawned child, when the platform exposes it.
    fn process_id(&self) -> Option<u32>;

    /// A writer onto the process's input for another thread, so a child that
    /// stops reading blocks that thread rather than the controller.
    fn input(&self) -> Option<Box<dyn ProcessInput>>;

    /// Whether something behind this handle already answers the child's
    /// terminal queries, so the supervisor must not answer them a second time.
    /// Defaults to no: a bare PTY leaves answering to its reader.
    fn answers_terminal_queries(&self) -> bool {
        false
    }

    /// Where the process's authoritative screen is read.
    fn screen_source(&self) -> ScreenSource;

    /// Lets go of the process for good, stopping it so nothing is left
    /// running unsupervised. It never blocks the caller.
    fn abandon(self: Box<Self>);

    /// The screen the process already had when this handle connected, to paint
    /// before any output that follows. Taken once; later calls return none.
    /// Defaults to none: a fresh PTY starts blank.
    fn take_initial_screen(&mut self) -> Option<Vec<u8>> {
        None
    }

    /// The PTY size of that screen, which a viewer of it must take rather than
    /// impose its own on a running process. Defaults to none: a fresh PTY has
    /// the size it was spawned with.
    fn screen_size(&self) -> Option<PtySize> {
        None
    }

    /// The writable clients attached to the process's terminal when this
    /// handle connected. Defaults to none attached.
    fn attached_clients(&self) -> u32 {
        0
    }

    /// When the process started, when that predates this handle. Defaults to
    /// none: it started as the handle was made.
    fn started_at(&self) -> Option<SystemTime> {
        None
    }

    /// Where the process's current run began in its logged output, when the
    /// output is logged with offsets. Defaults to none: a bare PTY logs
    /// nothing.
    fn run_start(&self) -> Option<OutputCursor> {
        None
    }

    /// Tells whatever holds the finished process that its exit was handled, so
    /// it can let go of it. Until then a holder keeps the exit for a later
    /// supervisor. Defaults to nothing to release.
    ///
    /// # Errors
    /// Returns a `PtyError` if the acknowledgement cannot be delivered.
    fn acknowledge_exit(&mut self) -> Result<(), PtyError> {
        Ok(())
    }

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

    /// This runner, shareable with a worker thread, when a spawn may block
    /// long enough that the caller must not wait for it on its own loop.
    /// Defaults to none: spawns are quick and run inline.
    fn detached(&self) -> Option<Arc<dyn ProcessRunner + Send + Sync>> {
        None
    }
}
