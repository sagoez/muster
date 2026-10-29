//! A cursor as a repaint has to rebuild it: where it is and what it writes
//! with.

use alacritty_terminal::vte::ansi::StandardCharset;
use getset::CopyGetters;
use typed_builder::TypedBuilder;

use super::emulator::CellStyle;

/// One cursor's position, pending wrap, pen and designated G0 and G1
/// charsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct CursorState {
    /// Zero-based `(row, col)`.
    position: (u16, u16),
    /// Whether the cursor sits past the last column of a just-filled line,
    /// so the next character starts the next line.
    wrap_pending: bool,
    /// The style new characters and erased cells take.
    pen: CellStyle,
    /// The G0 then G1 charsets.
    charsets: [StandardCharset; 2],
}
