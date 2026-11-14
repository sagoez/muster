use std::fmt::{self, Write};

/// Extended-color mode that picks one of 256 colors by index.
pub const PALETTE_MODE: u16 = 5;
/// Extended-color mode that gives red, green and blue.
pub const RGB_MODE: u16 = 2;
/// Separates SGR parameters.
pub const PARAM_SEPARATOR: char = ';';

/// One color a [`super::pen::Pen`] draws with, as SGR picks it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PenColor {
    /// One of the sixteen named colors, kept as the SGR code that picked it
    /// (`31`, `94`, `42`, ...).
    Named(u8),
    /// An index into the 256-color palette.
    Indexed(u8),
    /// A true color.
    Rgb(u8, u8, u8),
}

impl PenColor {
    /// Writes the SGR parameters that pick this color into `out`, an
    /// extended color under `extended` (38, 48 or 58).
    ///
    /// # Errors
    /// Returns a [`fmt::Error`] when `out` cannot be written.
    pub fn write_params(self, extended: u16, out: &mut impl Write) -> fmt::Result {
        let separator = PARAM_SEPARATOR;
        match self {
            Self::Named(code) => write!(out, "{code}"),
            Self::Indexed(index) => {
                write!(out, "{extended}{separator}{PALETTE_MODE}{separator}{index}")
            },
            Self::Rgb(red, green, blue) => write!(
                out,
                "{extended}{separator}{RGB_MODE}{separator}{red}{separator}{green}{separator}{blue}"
            ),
        }
    }

    /// The extended color `arguments` give after the mode that opens them:
    /// a palette index, or red, green and blue; none when they are missing
    /// or out of range.
    pub fn extended(mode: u16, arguments: &[u16]) -> Option<Self> {
        let byte = |value: &u16| u8::try_from(*value).ok();
        match (mode, arguments) {
            (PALETTE_MODE, [index, ..]) => Some(Self::Indexed(byte(index)?)),
            (RGB_MODE, [red, green, blue, ..]) => {
                Some(Self::Rgb(byte(red)?, byte(green)?, byte(blue)?))
            },
            _ => None,
        }
    }
}
