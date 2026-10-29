//! A process's raw output made safe to print on a person's terminal: text,
//! line breaks, colours, cursor moves and erases pass, so a progress bar
//! redraws, while anything that could query the terminal, retitle it or write
//! its clipboard is dropped.

use std::io::{self, Write};

use super::kept_bytes::Kept;
use crate::adapter::terminal::bounded_parser::BoundedParser;

/// A writer that passes what reaches it on to `out` with every terminal
/// string (OSC, DCS, APC, PM, SOS), C1 control, escape and control sequence
/// removed, except the colour (SGR), cursor-move, cursor-position and erase
/// sequences. A sequence split across writes is still recognised.
pub struct TerminalSafe<W: Write> {
    /// Where the safe bytes go.
    out: W,
    /// The parser, kept across writes.
    parser: BoundedParser,
    /// The safe bytes of the write being parsed.
    kept: Kept,
}

impl<W: Write> TerminalSafe<W> {
    /// A safe writer onto `out`.
    pub fn new(out: W) -> Self {
        Self {
            out,
            parser: BoundedParser::new(),
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
    /// a DCS, an APC, a reset and C1 controls do not.
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
        assert_eq!(safe(&[b"a\x1bcb"]), "ab");
        assert_eq!(safe(&["a\u{9b}6nb\u{9d}0;x\u{7}c".as_bytes()]), "a6nb0;xc");
        assert_eq!(safe(&["a\u{9b}2Kb".as_bytes()]), "a2Kb");
        assert_eq!(safe(&[b"a\x9b6n\x07b"]), "a6nb");
        assert_eq!(safe(&[b"\x1b[38:2:255:0:0mx"]), "\x1b[38:2:255:0:0mx");
    }

    /// A progress bar's carriage returns, cursor moves and erases pass, so it
    /// redraws in place.
    #[test]
    fn cursor_moves_and_erases_pass() {
        assert_eq!(
            safe(&[b"10%\r\x1b[2K20%\x1b[1A\x1b[2B\x1b[3C\x1b[4D\x1b[5G\x1b[2;7H\x1b[0J"]),
            "10%\r\x1b[2K20%\x1b[1A\x1b[2B\x1b[3C\x1b[4D\x1b[5G\x1b[2;7H\x1b[0J"
        );
        assert_eq!(safe(&[b"a\x1b[Kb"]), "a\x1b[0Kb");
    }

    /// Queries are dropped: status and cursor reports (DSR), device
    /// attributes (DA), a setting request (DECRQSS), window and title reports,
    /// a private-mode report, and a private-marker erase or mode change.
    #[test]
    fn terminal_queries_are_dropped() {
        assert_eq!(safe(&[b"a\x1b[5n\x1b[6n\x1b[?6nb"]), "ab");
        assert_eq!(safe(&[b"a\x1b[c\x1b[0c\x1b[>c\x1b[=cb"]), "ab");
        assert_eq!(safe(&[b"a\x1bP$q\"p\x1b\\b"]), "ab");
        assert_eq!(safe(&[b"a\x1b[18t\x1b[21t\x1b[14tb"]), "ab");
        assert_eq!(safe(&[b"a\x1b[?2026$pb"]), "ab");
        assert_eq!(safe(&[b"a\x1b[?2J\x1b[?25lb"]), "ab");
        assert_eq!(safe(&[b"a\x1bX sos \x1b\\b"]), "ab");
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
