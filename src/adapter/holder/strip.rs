//! Turning one raw output line into the plain text an agent reads: terminal
//! escape sequences removed and carriage returns applied as overwrites, so a
//! progress bar collapses to its last state.

use alacritty_terminal::vte::{Params, Parser, Perform};

/// Returns the cursor to the start of the line.
const CARRIAGE_RETURN: u8 = b'\r';
/// Moves the cursor one column left.
const BACKSPACE: u8 = b'\x08';
/// Moves to the next tab stop; kept as a tab so indentation survives.
const TAB: u8 = b'\t';
/// The final byte of the erase-in-line sequence.
const ERASE_IN_LINE: char = 'K';
/// Erase-in-line mode: from the cursor to the end.
const ERASE_TO_END: u16 = 0;
/// Erase-in-line mode: from the start to the cursor.
const ERASE_TO_CURSOR: u16 = 1;
/// Erase-in-line mode: the whole line.
const ERASE_LINE: u16 = 2;
/// What an erased column holds until something overwrites it.
const BLANK: char = ' ';
/// The final byte of the cursor-to-column sequence.
const CURSOR_TO_COLUMN: char = 'G';
/// The first column, as cursor-to-column numbers columns.
const FIRST_COLUMN: u16 = 1;
/// The furthest column a cursor-to-column may move to, so a sequence cannot
/// make a line allocate a huge run of blanks.
const MAX_COLUMN: usize = 1024;

/// One line as a terminal would leave it: a row of characters and the column
/// the next one lands in.
#[derive(Default)]
struct LineScreen {
    /// The characters of the line.
    cells: Vec<char>,
    /// The column the next printed character lands in.
    cursor: usize,
}

impl LineScreen {
    /// Puts `character` at the cursor, overwriting what is there and padding
    /// with blanks when the cursor was moved past the end.
    fn put(&mut self, character: char) {
        if self.cursor >= self.cells.len() {
            self.cells.resize(self.cursor, BLANK);
            self.cells.push(character);
        } else {
            self.cells[self.cursor] = character;
        }
        self.cursor += 1;
    }

    /// Applies an erase-in-line sequence in `mode`; erasing to the cursor
    /// includes the cursor's own column, as a terminal does.
    fn erase(&mut self, mode: u16) {
        let length = self.cells.len();
        match mode {
            ERASE_TO_END => self.cells.truncate(self.cursor.min(length)),
            ERASE_TO_CURSOR => self.cells[..(self.cursor + 1).min(length)].fill(BLANK),
            ERASE_LINE => self.cells.clear(),
            _ => {},
        }
    }
}

impl Perform for LineScreen {
    /// Prints a character at the cursor.
    fn print(&mut self, character: char) {
        self.put(character);
    }

    /// Applies the control characters that shape a line; others are dropped.
    fn execute(&mut self, byte: u8) {
        match byte {
            CARRIAGE_RETURN => self.cursor = 0,
            BACKSPACE => self.cursor = self.cursor.saturating_sub(1),
            TAB => self.put(char::from(TAB)),
            _ => {},
        }
    }

    /// Applies erase-in-line and cursor-to-column, the sequences progress
    /// output uses to redraw a line in place; every other sequence is dropped.
    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
        if !intermediates.is_empty() {
            return;
        }
        let first = params
            .iter()
            .next()
            .and_then(|param| param.first().copied());
        match action {
            ERASE_IN_LINE => self.erase(first.unwrap_or(ERASE_TO_END)),
            CURSOR_TO_COLUMN => {
                let column = usize::from(first.unwrap_or(FIRST_COLUMN).max(FIRST_COLUMN));
                self.cursor = column.min(MAX_COLUMN) - usize::from(FIRST_COLUMN);
            },
            _ => {},
        }
    }
}

/// The text of one raw output line, without its line feed: escape sequences
/// removed, carriage returns, backspaces and erase-in-line applied as a
/// terminal would, invalid UTF-8 replaced and trailing whitespace trimmed.
pub fn strip_line(raw: &[u8]) -> String {
    let mut screen = LineScreen::default();
    Parser::new().advance(&mut screen, raw);
    let text: String = screen.cells.into_iter().collect();
    text.trim_end().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_sequences_are_removed() {
        assert_eq!(
            strip_line(b"\x1b[1;32mready\x1b[0m in 3ms\r"),
            "ready in 3ms"
        );
        assert_eq!(
            strip_line(b"\x1b]0;title\x07Listening on :3000"),
            "Listening on :3000"
        );
        assert_eq!(strip_line(b"bell\x07 and \x1b(0line"), "bell and line");
    }

    /// A progress bar redrawn with carriage returns reads as its last state.
    #[test]
    fn carriage_returns_overwrite() {
        assert_eq!(strip_line(b"10%\r50%\r100% done"), "100% done");
        assert_eq!(strip_line(b"downloading 1/3\r\x1b[Kok"), "ok");
        assert_eq!(strip_line(b"abcdef\rXY"), "XYcdef");
        assert_eq!(strip_line(b"line\r\n"), "line");
        assert_eq!(strip_line(b"abc\x08\x08Z"), "aZc");
        assert_eq!(strip_line(b"abcdef\rabc\x1b[1K!"), "   !ef");
        assert_eq!(strip_line(b"abc\x1b[2K"), "");
    }

    /// A spinner that returns with cursor-to-column redraws in place, and a
    /// column far past the line is clamped rather than padded without end.
    #[test]
    fn cursor_to_column_moves_within_the_line() {
        assert_eq!(strip_line(b"\x1b[1G- working\x1b[1G\x1b[0Kdone"), "done");
        assert_eq!(strip_line(b"ab\x1b[5Gc"), "ab  c");
        assert_eq!(strip_line(b"\x1b[65535Gx").chars().count(), MAX_COLUMN);
    }

    #[test]
    fn tabs_survive_and_invalid_utf8_is_replaced() {
        assert_eq!(
            strip_line(b"\tat Foo.bar(Foo.java:3)"),
            "\tat Foo.bar(Foo.java:3)"
        );
        assert_eq!(strip_line(b"caf\xc3\xa9 \xff!"), "caf\u{e9} \u{fffd}!");
    }
}
