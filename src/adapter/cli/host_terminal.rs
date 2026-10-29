use std::{
    io::{self, IsTerminal},
    os::fd::{AsFd, OwnedFd},
};

use rustix::termios::tcgetwinsize;

use super::error::CliError;
use crate::domain::{
    pty::PtySize,
    value::{Cols, Rows},
};

/// The terminal `muster attach` takes over: where keys are read from and the
/// process's screen is drawn. An RAII resource owning duplicated file
/// descriptors, closed on drop, so it has a hand-written constructor.
pub struct HostTerminal {
    /// Where keys are read from.
    input: OwnedFd,
    /// Where the screen is drawn.
    output: OwnedFd,
}

impl HostTerminal {
    /// This process's standard input and output, when both are terminals.
    ///
    /// # Errors
    /// Returns [`CliError::NotATerminal`] when either is not a terminal, or
    /// [`CliError::Terminal`] when they cannot be duplicated.
    pub fn stdio() -> Result<Self, CliError> {
        let (stdin, stdout) = (io::stdin(), io::stdout());
        if !stdin.is_terminal() || !stdout.is_terminal() {
            return Err(CliError::NotATerminal);
        }
        Ok(Self::new(
            stdin
                .as_fd()
                .try_clone_to_owned()
                .map_err(CliError::Terminal)?,
            stdout
                .as_fd()
                .try_clone_to_owned()
                .map_err(CliError::Terminal)?,
        ))
    }

    /// The terminal read from `input` and drawn on `output`.
    pub fn new(input: OwnedFd, output: OwnedFd) -> Self {
        Self { input, output }
    }

    /// Where keys are read from.
    pub fn input(&self) -> &OwnedFd {
        &self.input
    }

    /// Where the screen is drawn.
    pub fn output(&self) -> &OwnedFd {
        &self.output
    }

    /// The terminal's size pulled within the PTY bounds, since that is the
    /// most a holder takes, or none when it reports no rows or columns.
    pub fn size(&self) -> Option<PtySize> {
        let size = tcgetwinsize(&self.output).ok()?;
        (size.ws_row > 0 && size.ws_col > 0).then(|| {
            PtySize::builder()
                .rows(Rows::new(size.ws_row))
                .cols(Cols::new(size.ws_col))
                .build()
                .clamped()
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        adapter::cli::test_pty::TestPty,
        domain::{
            pty::PtySize,
            value::{Cols, Rows},
        },
    };

    /// A one-column terminal is reported at the narrowest size a holder
    /// takes, so an attach never sends a resize the holder refuses.
    #[test]
    fn a_one_column_terminal_is_reported_within_the_pty_bounds() {
        let pty = TestPty::open(24, 1);
        assert_eq!(
            pty.terminal().size(),
            Some(
                PtySize::builder()
                    .rows(Rows::new(24))
                    .cols(Cols::new(PtySize::MIN_COLS))
                    .build()
            )
        );
    }
}
