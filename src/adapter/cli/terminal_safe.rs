//! A process's raw output made safe to print on a person's terminal: text,
//! line breaks and colours pass, while anything that could query the
//! terminal, retitle it, write its clipboard or move its cursor is dropped.

use std::io::{self, Write};

use vte::{Params, Parser, Perform};

/// The control characters that only lay text out, kept as they are: line
/// feed, carriage return, tab and backspace.
const LAYOUT_CONTROLS: &[u8] = b"\n\r\t\x08";
/// The final character of a select-graphic-rendition (colour) sequence.
const SGR_FINAL: char = 'm';
/// Opens a control sequence.
const CSI: &[u8] = b"\x1b[";
/// Separates a control sequence's parameters.
const PARAM_SEPARATOR: char = ';';
/// Separates the sub-parameters of one parameter.
const SUBPARAM_SEPARATOR: char = ':';

/// A writer that passes what reaches it on to `out` with every terminal
/// string (OSC, DCS, APC, PM, SOS), C1 control, escape and control sequence
/// removed, except colour (SGR) sequences. A sequence split across writes is
/// still recognised.
pub struct TerminalSafe<W: Write> {
    /// Where the safe bytes go.
    out: W,
    /// The parser, kept across writes.
    parser: Parser,
    /// The safe bytes of the write being parsed.
    kept: Kept,
}

impl<W: Write> TerminalSafe<W> {
    /// A safe writer onto `out`.
    pub fn new(out: W) -> Self {
        Self {
            out,
            parser: Parser::new(),
            kept: Kept::default(),
        }
    }
}

impl<W: Write> Write for TerminalSafe<W> {
    /// Parses `bytes` and writes on what is safe to show.
    ///
    /// # Errors
    /// Returns the [`io::Error`] of a failed write to the inner writer.
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.parser.advance(&mut self.kept, bytes);
        let written = self.out.write_all(&self.kept.bytes);
        self.kept.bytes.clear();
        written.map(|()| bytes.len())
    }

    /// Flushes the inner writer.
    ///
    /// # Errors
    /// Returns the [`io::Error`] of a failed flush.
    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

/// The bytes a parse keeps.
#[derive(Default)]
struct Kept {
    /// What is safe to show, in order.
    bytes: Vec<u8>,
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

    /// Keeps a colour sequence, rebuilt from its parameters; drops every
    /// other control sequence, such as a device query or a cursor move.
    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        if ignore || action != SGR_FINAL || !intermediates.is_empty() {
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
        self.bytes.extend_from_slice(CSI);
        self.bytes
            .extend_from_slice(rebuilt.join(&PARAM_SEPARATOR.to_string()).as_bytes());
        self.bytes.push(SGR_FINAL as u8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What writing each of `chunks` in turn passes on.
    fn safe(chunks: &[&[u8]]) -> String {
        let mut out = Vec::new();
        let mut writer = TerminalSafe::new(&mut out);
        for chunk in chunks {
            writer.write_all(chunk).unwrap();
        }
        String::from_utf8(out).unwrap()
    }

    /// Colours and layout pass; a title, a clipboard write, a device query,
    /// a DCS, an APC, a cursor move and C1 controls do not.
    #[test]
    fn only_text_layout_and_colour_pass() {
        assert_eq!(
            safe(&[b"\x1b[1;31mred\x1b[0m\tok\r\n"]),
            "\x1b[1;31mred\x1b[0m\tok\r\n"
        );
        assert_eq!(safe(&[b"a\x1b]0;pwned\x07b"]), "ab");
        assert_eq!(safe(&[b"a\x1b]52;c;aGk=\x1b\\b"]), "ab");
        assert_eq!(safe(&[b"a\x1b[6nb\x1b[cc\x1b[>cd"]), "abcd");
        assert_eq!(safe(&[b"a\x1bP1$qm\x1b\\b"]), "ab");
        assert_eq!(safe(&[b"a\x1b_payload\x1b\\b\x1b^pm\x1b\\c"]), "abc");
        assert_eq!(safe(&[b"a\x1b[2J\x1b[Hb\x1bcc"]), "abc");
        assert_eq!(safe(&["a\u{9b}6nb\u{9d}0;x\u{7}c".as_bytes()]), "a6nb0;xc");
        assert_eq!(safe(&[b"a\x9b6n\x07b"]), "a6nb");
        assert_eq!(safe(&[b"\x1b[38:2:255:0:0mx"]), "\x1b[38:2:255:0:0mx");
    }

    /// A sequence split across two writes is still removed whole.
    #[test]
    fn a_split_sequence_is_still_removed() {
        assert_eq!(safe(&[b"a\x1b]0;pw", b"ned\x07b"]), "ab");
        assert_eq!(
            safe(&[b"caf\xc3", b"\xa9 \x1b[3", b"2mok"]),
            "caf\u{e9} \x1b[32mok"
        );
    }
}
