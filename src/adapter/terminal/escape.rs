//! The escape-sequence vocabulary muster's VT scanners recognize and the
//! sequences it writes share.

use alacritty_terminal::vte::Params;

/// Escape byte that introduces every sequence.
pub const ESC: u8 = 0x1B;
/// Control Sequence Introducer.
pub const CSI: &str = "\x1b[";
/// Intermediate that marks a DEC private sequence (`CSI ? ...`).
pub const PRIVATE_MARKER: u8 = b'?';
/// Final of a sequence that sets modes (`CSI ? n h`).
pub const SET_MODE_FINAL: char = 'h';
/// Final of a sequence that resets modes (`CSI ? n l`).
pub const RESET_MODE_FINAL: char = 'l';
/// Final of DECSTBM, which sets the scroll region.
pub const SCROLL_REGION_FINAL: char = 'r';
/// Final of a Kitty keyboard sequence (`CSI ... u`).
pub const KITTY_FINAL: char = 'u';
/// Final of RIS (`ESC c`), a full terminal reset.
pub const FULL_RESET_FINAL: u8 = b'c';
/// DEC private modes that switch to and from the alternate screen.
pub const ALT_SCREEN_MODES: [u16; 3] = [47, 1047, 1049];
/// SGR that resets every text attribute.
pub const SGR_RESET: &str = "\x1b[0m";

/// The value of each parameter in `params`, without its sub-parameters; an
/// omitted parameter reads as zero.
pub fn param_values(params: &Params) -> impl Iterator<Item = u16> + '_ {
    params.iter().filter_map(|param| param.first().copied())
}
