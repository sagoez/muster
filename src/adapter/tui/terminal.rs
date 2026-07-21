use std::io::{self, Stdout};

use crossterm::{
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use getset::MutGetters;
use ratatui::{Terminal, backend::CrosstermBackend};

use crate::error::Result;

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
    /// Enters raw mode and the alternate screen, returning a ready terminal.
    ///
    /// # Errors
    /// Returns an error if raw mode cannot be enabled, the alternate screen
    /// cannot be entered, or the terminal backend fails to initialize.
    pub fn new() -> Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen) {
            let _ = disable_raw_mode();
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

    /// Restores the terminal to its original cooked state. Safe to call more
    /// than once; used by both `Drop` and the panic hook.
    ///
    /// # Errors
    /// Returns an error if raw mode cannot be disabled or the alternate screen
    /// cannot be left.
    pub fn restore() -> io::Result<()> {
        let raw = disable_raw_mode();
        let screen = execute!(io::stdout(), LeaveAlternateScreen);
        raw.and(screen)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = Self::restore();
    }
}
