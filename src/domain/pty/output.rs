/// The exit disposition of a process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitOutcome {
    /// Exited with status zero.
    Succeeded,
    /// Exited with a non-zero status or was killed by a signal.
    Failed,
}

/// An event emitted by a running process, delivered to the runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessOutput {
    /// A chunk of raw bytes read from the process's PTY.
    Chunk(Vec<u8>),
    /// The process exited with the given outcome.
    Exited(ExitOutcome),
}
