use std::io::{self, Stdout};

use crossterm::{
    clipboard::CopyToClipboard,
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    style::Print,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use getset::MutGetters;
use ratatui::{Terminal, backend::CrosstermBackend};

use super::{clipboard, pointer_shape::PointerShape};
use crate::error::Result;

/// OSC prefix that sets the host pointer shape (xterm OSC 22).
const POINTER_SHAPE_PREFIX: &str = "\x1b]22;";
/// String terminator closing an OSC sequence.
const OSC_TERMINATOR: &str = "\x1b\\";

/// The concrete ratatui terminal type: a crossterm backend on stdout.
pub type Tui = Terminal<CrosstermBackend<Stdout>>;

/// RAII guard that enters raw mode + the alternate screen on construction and
/// restores the original terminal state on drop.
#[derive(MutGetters)]
pub struct TerminalGuard {
    #[getset(get_mut = "pub")]
    terminal: Tui,
}

impl TerminalGuard {
    /// Enters raw mode and the alternate screen, and captures the mouse so
    /// Muster owns pane-scoped selection and scrolling.
    ///
    /// # Errors
    /// Returns an error if raw mode cannot be enabled, the alternate screen
    /// cannot be entered, or the terminal backend fails to initialize.
    pub fn new() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen, EnableMouseCapture) {
            let _ = Self::restore();
            return Err(error.into());
        }
        match Terminal::new(CrosstermBackend::new(stdout)) {
            Ok(terminal) => Ok(Self { terminal }),
            Err(error) => {
                let _ = Self::restore();
                Err(error.into())
            },
        }
    }

    /// Copies `text` onto the system clipboard: a native clipboard tool when
    /// one works (herdr's preference), falling back to OSC 52 through the
    /// host terminal.
    ///
    /// # Errors
    /// Returns an error when the escape sequence cannot be written to stdout.
    pub fn copy_to_clipboard(&mut self, text: &str) -> io::Result<()> {
        if clipboard::write_native(text) {
            return Ok(());
        }
        execute!(io::stdout(), CopyToClipboard::to_clipboard_from(text))
    }

    /// Asks the host terminal to show `shape` as the mouse pointer (OSC 22).
    ///
    /// # Errors
    /// Returns an error when the escape sequence cannot be written to stdout.
    pub fn set_pointer_shape(&mut self, shape: PointerShape) -> io::Result<()> {
        execute!(
            io::stdout(),
            Print(format!("{POINTER_SHAPE_PREFIX}{shape}{OSC_TERMINATOR}"))
        )
    }

    /// Restores the terminal to its original cooked state. Safe to call more
    /// than once; used by both `Drop` and the panic hook.
    ///
    /// # Errors
    /// Returns an error if raw mode cannot be disabled or the alternate screen
    /// cannot be left.
    pub fn restore() -> io::Result<()> {
        let raw = disable_raw_mode();
        let mouse = execute!(io::stdout(), DisableMouseCapture);
        let pointer = execute!(
            io::stdout(),
            Print(format!(
                "{POINTER_SHAPE_PREFIX}{}{OSC_TERMINATOR}",
                PointerShape::Default
            ))
        );
        let screen = execute!(io::stdout(), LeaveAlternateScreen);
        raw.and(mouse).and(pointer).and(screen)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = Self::restore();
    }
}
