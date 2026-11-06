use std::{fmt, fmt::Write};

use super::pen_color::PARAM_SEPARATOR;

/// Writes the separator before every SGR parameter but the first.
#[derive(Default)]
pub struct Separator {
    /// Whether a parameter was written already.
    started: bool,
}

impl Separator {
    /// Writes a separator into `out` unless this is the first parameter.
    ///
    /// # Errors
    /// Returns a [`fmt::Error`] when `out` cannot be written.
    pub fn write(&mut self, out: &mut impl Write) -> fmt::Result {
        if self.started {
            out.write_char(PARAM_SEPARATOR)?;
        }
        self.started = true;
        Ok(())
    }
}
