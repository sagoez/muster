use alacritty_terminal::vte::{Params, Perform};

use crate::adapter::terminal::escape::CSI;

/// The control characters that only lay text out, kept as they are: line
/// feed, carriage return, tab and backspace.
const LAYOUT_CONTROLS: &[u8] = b"\n\r\t\x08";
/// The final characters of the control sequences kept: colour (SGR), cursor
/// up, down, forward and back (CUU, CUD, CUF, CUB), cursor to column (CHA),
/// cursor position (CUP), erase in line (EL) and erase in display (ED).
const KEPT_FINALS: &[char] = &['m', 'A', 'B', 'C', 'D', 'G', 'H', 'K', 'J'];
/// Separates a control sequence's parameters.
const PARAM_SEPARATOR: char = ';';
/// Separates the sub-parameters of one parameter.
const SUBPARAM_SEPARATOR: char = ':';

/// The bytes a parse keeps.
#[derive(Default)]
pub(super) struct Kept {
    /// What is safe to show, in order.
    bytes: Vec<u8>,
}

impl Kept {
    /// The bytes kept since the last [`Kept::clear`].
    pub(super) fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Forgets the bytes kept, keeping the buffer for the next parse.
    pub(super) fn clear(&mut self) {
        self.bytes.clear();
    }
}

impl Perform for Kept {
    /// Keeps a printed character.
    fn print(&mut self, character: char) {
        let mut encoded = [0; 4];
        self.bytes
            .extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
    }

    /// Keeps the controls that lay text out; drops every other C0 and C1.
    fn execute(&mut self, byte: u8) {
        if LAYOUT_CONTROLS.contains(&byte) {
            self.bytes.push(byte);
        }
    }

    /// Keeps a colour, cursor-move or erase sequence, rebuilt from its
    /// parameters; drops every other control sequence, such as a device,
    /// status or window query, and any with a private marker.
    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        if ignore || !KEPT_FINALS.contains(&action) || !intermediates.is_empty() {
            return;
        }
        let rebuilt: Vec<String> = params
            .iter()
            .map(|param| {
                param
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(&SUBPARAM_SEPARATOR.to_string())
            })
            .collect();
        self.bytes.extend_from_slice(CSI.as_bytes());
        self.bytes
            .extend_from_slice(rebuilt.join(&PARAM_SEPARATOR.to_string()).as_bytes());
        let mut encoded = [0; 4];
        self.bytes
            .extend_from_slice(action.encode_utf8(&mut encoded).as_bytes());
    }
}
