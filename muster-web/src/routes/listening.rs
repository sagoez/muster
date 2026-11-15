use std::io::{self, Write};

use crate::http::StreamOut;

/// A client still there, keeping what is written to it and counting each
/// time it is asked whether it hung up.
#[derive(Default)]
pub(crate) struct Listening {
    /// Everything written to it.
    written: Vec<u8>,
    /// How many times it was asked whether it hung up.
    asked: usize,
}

impl Listening {
    /// How many times it was asked whether it hung up.
    pub(crate) fn asked(&self) -> usize {
        self.asked
    }

    /// Everything written to it, as text.
    pub(crate) fn text(&self) -> String {
        String::from_utf8_lossy(&self.written).into_owned()
    }
}

impl Write for Listening {
    /// Keeps `written` whole.
    fn write(&mut self, written: &[u8]) -> io::Result<usize> {
        self.written.write(written)
    }

    /// Flushes nothing.
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl StreamOut for Listening {
    /// Never hung up; counts the ask.
    fn hung_up(&mut self) -> bool {
        self.asked += 1;
        false
    }
}
