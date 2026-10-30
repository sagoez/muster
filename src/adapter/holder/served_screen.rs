use std::path::PathBuf;

use super::client::screen_text;
use crate::domain::{port::ScreenReader, pty::PtyError};

/// The screen a holder serves on its socket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServedScreen {
    /// The socket of the holder that serves it.
    socket: PathBuf,
}

impl ServedScreen {
    /// The screen served by the holder at `socket`.
    pub fn new(socket: PathBuf) -> Self {
        Self { socket }
    }
}

impl ScreenReader for ServedScreen {
    /// Reads the visible screen from the holder, blocking for at most the
    /// holder client's read timeout.
    fn read_screen(&self) -> Result<String, PtyError> {
        screen_text(&self.socket)
    }
}
