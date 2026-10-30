use super::{exit_outcome::ExitOutcome, size::PtySize};
use crate::domain::value::OutputCursor;

/// An event emitted by a running process, delivered to the runtime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessOutput {
    /// A chunk of raw bytes read from the process's PTY.
    Chunk(Vec<u8>),
    /// A chunk whose first byte sits at `at` in the process's logged output,
    /// so a receiver that missed chunks sees the gap.
    Placed {
        /// Where `bytes` start in the output stream.
        at: OutputCursor,
        /// The bytes.
        bytes: Vec<u8>,
    },
    /// Bytes that rebuild the process's screen in a fresh terminal of `size`:
    /// a repaint, not output the process wrote.
    Repaint {
        /// The bytes that rebuild the screen.
        bytes: Vec<u8>,
        /// The size the screen was rendered at.
        size: PtySize,
    },
    /// The process exited with the given outcome.
    Exited(ExitOutcome),
    /// The stream carrying the process's output could not be read any further,
    /// for the given reason. The process itself may still be running.
    Unreadable(String),
    /// Whatever runs the process recovered from a fault the user should know
    /// about, described.
    Warning(String),
    /// The writable clients now attached to the process's terminal.
    Attached(u32),
}
