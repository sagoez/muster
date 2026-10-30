use std::io::{self, Write};

use crate::domain::value::OutputCursor;

/// Where `muster logs -f` writes a followed log: its bytes in order, and a
/// gap wherever a rotation discarded output before it was read.
pub trait FollowedOutput: Write {
    /// Notes that `skipped` bytes were rotated away unread, so the next byte
    /// written sits at `resumed` in the process's output.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when the note cannot be written.
    fn gap(&mut self, skipped: u64, resumed: OutputCursor) -> io::Result<()>;
}
