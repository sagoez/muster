use std::time::{Duration, SystemTime};

use super::ProcessInput;
use crate::domain::{
    process::StopSignal,
    pty::{PtyError, PtySize, ScreenSource},
    value::{OutputCursor, Pid},
};

/// Handle to a spawned process: write input, resize, or terminate it.
pub trait ProcessHandle: Send {
    /// The OS process ID of the spawned child, when the platform exposes it.
    fn process_id(&self) -> Option<Pid>;

    /// A writer onto the process's input for another thread, so a child that
    /// stops reading blocks that thread rather than the controller.
    fn input(&self) -> Option<Box<dyn ProcessInput>>;

    /// Where the process's authoritative screen is read.
    fn screen_source(&self) -> ScreenSource;

    /// Lets go of the process for good, stopping it so nothing is left
    /// running unsupervised. It never blocks the caller.
    fn abandon(self: Box<Self>);

    /// The screen the process already had when this handle connected, to paint
    /// before any output that follows. Taken once; later calls return none,
    /// as does a fresh PTY, which starts blank.
    fn take_initial_screen(&mut self) -> Option<Vec<u8>>;

    /// The PTY size of that screen, which a viewer of it must take rather than
    /// impose its own on a running process; none for a fresh PTY, which has
    /// the size it was spawned with.
    fn screen_size(&self) -> Option<PtySize>;

    /// The writable clients attached to the process's terminal when this
    /// handle connected.
    fn attached_clients(&self) -> u32;

    /// When the process started, when that predates this handle; none when it
    /// started as the handle was made.
    fn started_at(&self) -> Option<SystemTime>;

    /// Where the process's current run began in its logged output; none when
    /// the output is not logged with offsets.
    fn run_start(&self) -> Option<OutputCursor>;

    /// Where the process's logged output ends now, read from its log; none
    /// when the output is not logged or the log cannot be read. It may run
    /// ahead of the output this handle has delivered.
    fn output_end(&self) -> Option<OutputCursor>;

    /// The logged output the process wrote before this handle connected, and
    /// where it starts, so every byte is seen before the output that follows.
    /// Taken once; none when every byte is streamed.
    fn take_backlog(&mut self) -> Option<(OutputCursor, Vec<u8>)>;

    /// Tells whatever holds the finished process that its exit was handled, so
    /// it can let go of it. Until then a holder keeps the exit for a later
    /// supervisor.
    ///
    /// # Errors
    /// Returns a `PtyError` if the acknowledgement cannot be delivered.
    fn acknowledge_exit(&mut self) -> Result<(), PtyError>;

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

    /// Requests graceful process termination with `signal`, giving it `grace`
    /// before a kill.
    ///
    /// # Errors
    /// Returns a `PtyError` if the termination signal cannot be sent.
    fn terminate(&mut self, signal: StopSignal, grace: Duration) -> Result<(), PtyError>;

    /// Forcibly terminates the process.
    ///
    /// # Errors
    /// Returns a `PtyError` if the kill signal cannot be sent.
    fn kill(&mut self) -> Result<(), PtyError>;
}
