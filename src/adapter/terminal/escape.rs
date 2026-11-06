//! The escape-sequence vocabulary muster's VT scanners recognize and the
//! sequences it writes share.

use alacritty_terminal::vte::Params;

/// Escape byte that introduces every sequence.
pub const ESC: u8 = 0x1B;
/// BEL, which also ends an OSC string.
pub const BEL: u8 = 0x07;
/// CAN, which cancels any sequence in progress.
pub const CAN: u8 = 0x18;
/// SUB, which cancels any sequence in progress.
pub const SUB: u8 = 0x1A;
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
/// Final of SGR (`CSI ... m`), which sets the pen.
pub const SGR_FINAL: char = 'm';
/// Intermediate of the sequences that push and pop the pen (`CSI # {`).
pub const PEN_STACK_INTERMEDIATE: u8 = b'#';
/// Final of XTPUSHSGR, which saves the pen on a stack.
pub const PUSH_PEN_FINAL: char = '{';
/// Final of XTPOPSGR, which restores the pen last pushed.
pub const POP_PEN_FINAL: char = '}';
/// XTPUSHSGR: saves the pen, across line feeds, for [`POP_PEN`].
pub const PUSH_PEN: &[u8] = b"\x1b[#{";
/// XTPOPSGR: restores the pen [`PUSH_PEN`] saved.
pub const POP_PEN: &[u8] = b"\x1b[#}";
/// Ends a line of output.
pub const LINE_FEED: u8 = b'\n';
/// Ends the line a log was drawing, when one is open, as the holder writes
/// it into a log: CAN cancels a sequence cut short, a cursor save marks the
/// line as a redraw, so a blank one is dropped rather than kept, and a line
/// break ends it.
pub const END_OPEN_LINE: &[u8] = b"\x18\x1b7\r\n";
/// What the holder logs where each run starts, before the child's first
/// byte: [`END_OPEN_LINE`], then a full reset (RIS), so every reader renders
/// each run from a fresh terminal without knowing where it began.
pub const RUN_START: &[u8] = b"\x18\x1b7\r\n\x1bc";

/// The value of each parameter in `params`, without its sub-parameters; an
/// omitted parameter reads as zero.
pub fn param_values(params: &Params) -> impl Iterator<Item = u16> + '_ {
    params.iter().filter_map(|param| param.first().copied())
}
