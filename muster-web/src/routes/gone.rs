use std::io::{self, Write};

use crate::http::StreamOut;

/// A client that has hung up and takes nothing written to it.
pub(crate) struct Gone;

impl Write for Gone {
    /// Takes `written` whole, as a socket does before it learns the peer left.
    fn write(&mut self, written: &[u8]) -> io::Result<usize> {
        Ok(written.len())
    }

    /// Flushes nothing.
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl StreamOut for Gone {
    /// Always hung up.
    fn hung_up(&mut self) -> bool {
        true
    }
}
