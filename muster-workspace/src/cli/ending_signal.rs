use std::{ffi::c_int, io};

use signal_hook::{
    consts::{SIGHUP, SIGTERM},
    low_level::emulate_default_handler,
};

/// A signal that ends `muster attach` early, leaving the process running.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndingSignal {
    /// The terminal hung up.
    Hangup,
    /// The client was asked to stop.
    Terminate,
}

impl EndingSignal {
    /// The signal's number.
    pub fn number(self) -> c_int {
        match self {
            Self::Hangup => SIGHUP,
            Self::Terminate => SIGTERM,
        }
    }

    /// Ends this process by the signal, as it would have ended without a
    /// session catching it, so its parent sees the signal.
    ///
    /// # Errors
    /// Returns the I/O error when the signal cannot be raised.
    pub fn resend(self) -> io::Result<()> {
        emulate_default_handler(self.number())
    }

    /// The ending signal numbered `number`, when it is one.
    pub fn from_number(number: c_int) -> Option<Self> {
        match number {
            SIGHUP => Some(Self::Hangup),
            SIGTERM => Some(Self::Terminate),
            _ => None,
        }
    }
}
