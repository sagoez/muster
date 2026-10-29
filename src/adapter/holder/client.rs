//! The client side of the holder protocol shared by the service's supervisor
//! connection, one-shot readers and `muster attach`.

use std::{os::unix::net::UnixStream, path::Path, time::Duration};

use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use super::{
    client_message::ClientMessage, client_role::ClientRole, error::HolderError,
    protocol::HolderMessage,
};
use crate::{
    adapter::{ipc, terminal::TerminalEmulator},
    domain::{
        color_scheme::ColorScheme,
        pty::{PtyError, PtySize},
        value::{OutputCursor, Pid},
    },
};

/// Longest a one-shot reader waits for the holder's screen.
const SCREEN_READ_TIMEOUT: Duration = Duration::from_secs(5);
/// Scrollback of the emulator that rebuilds a screen; only the visible screen
/// is read from it.
const SCREEN_SCROLLBACK: usize = 0;
/// Color scheme of the emulator that rebuilds a screen. It has no child, so it
/// answers no color query and either scheme reads the same text.
const SCREEN_COLOR_SCHEME: ColorScheme = ColorScheme::Dark;

/// The screen a holder sends a client that says hello.
#[derive(Debug, Getters, CopyGetters, TypedBuilder)]
pub struct Greeting {
    /// Bytes that rebuild the screen in a fresh terminal.
    #[getset(get = "pub")]
    repaint: Vec<u8>,
    /// The size the screen was rendered at.
    #[getset(get_copy = "pub")]
    size: PtySize,
    /// The child's process id.
    #[getset(get_copy = "pub")]
    pid: Pid,
    /// The writable attach clients connected.
    #[getset(get_copy = "pub")]
    attached: u32,
    /// Where the child's run began in its output stream.
    #[getset(get_copy = "pub")]
    run_start: OutputCursor,
    /// Where the log ended as the screen was taken; streamed output starts
    /// there.
    #[getset(get_copy = "pub")]
    log_end: OutputCursor,
}

impl Greeting {
    /// The bytes that rebuild the screen, taken out of the greeting.
    pub fn into_repaint(self) -> Vec<u8> {
        self.repaint
    }
}

/// Introduces `stream` to the holder at `socket` as `role` and reads the
/// screen it answers with, waiting at most `timeout`. The read timeout stays
/// set; a caller that keeps reading clears it.
///
/// # Errors
/// Returns [`HolderError::NoSnapshot`] when the holder answers with anything
/// but its screen or closes first, or the I/O error from the exchange.
pub fn greet(
    stream: &mut UnixStream,
    socket: &Path,
    role: ClientRole,
    timeout: Duration,
) -> Result<Greeting, HolderError> {
    stream.set_read_timeout(Some(timeout))?;
    ClientMessage::Hello { role }.write_to(stream)?;
    match HolderMessage::read_from(stream) {
        Ok(HolderMessage::Snapshot {
            repaint,
            size,
            pid,
            attached,
            run_start,
            log_end,
        }) => Ok(Greeting::builder()
            .repaint(repaint)
            .size(size)
            .pid(pid)
            .attached(attached)
            .run_start(run_start)
            .log_end(log_end)
            .build()),
        Ok(_) | Err(HolderError::Io(_)) => Err(HolderError::NoSnapshot(socket.to_path_buf())),
        Err(error) => Err(error),
    }
}

/// Reads the visible screen of the holder serving `socket`, as a read-only
/// client that leaves the child and every other client untouched.
///
/// # Errors
/// Returns a [`PtyError`] when the holder cannot be reached or sends no screen.
pub fn screen_text(socket: &Path) -> Result<String, PtyError> {
    let mut stream = ipc::connect(socket)?;
    let greeting = greet(
        &mut stream,
        socket,
        ClientRole::Attach { read_only: true },
        SCREEN_READ_TIMEOUT,
    )?;
    Ok(repaint_text(greeting.repaint(), greeting.size()))
}

/// The visible text of the screen `repaint` rebuilds in a fresh terminal of
/// `size`.
pub fn repaint_text(repaint: &[u8], size: PtySize) -> String {
    let mut screen = TerminalEmulator::new(
        size.rows(),
        size.cols(),
        SCREEN_SCROLLBACK,
        SCREEN_COLOR_SCHEME,
    );
    screen.process(repaint);
    screen.screen_text()
}
