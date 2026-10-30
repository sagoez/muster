//! What a process's output leaves on each line, worked out the way a log
//! reader expects rather than the way a screen redraws: one line at a time,
//! with carriage returns, backspaces, column moves and erases applied to it,
//! and everything that only redraws the screen around it dropped.

use std::mem;

use alacritty_terminal::vte::{Params, Perform};

use super::{
    escape::{ALT_SCREEN_MODES, PRIVATE_MARKER, RESET_MODE_FINAL, SET_MODE_FINAL, param_values},
    line_cell::LineCell,
    log_line::LogLine,
    pen::Pen,
};

/// Ends a line.
const LINE_FEED: u8 = b'\n';
/// Returns the cursor to the start of the line.
const CARRIAGE_RETURN: u8 = b'\r';
/// Moves the cursor one column left.
const BACKSPACE: u8 = b'\x08';
/// Moves to the next tab stop; kept as a tab so indentation survives.
const TAB: u8 = b'\t';
/// What an erased column holds until something overwrites it.
const BLANK: char = ' ';
/// The final byte of SGR, which sets the pen.
const SGR: char = 'm';
/// The final byte of erase in line (EL).
const ERASE_IN_LINE: char = 'K';
/// The final byte of erase in display (ED), applied to the line alone.
const ERASE_IN_DISPLAY: char = 'J';
/// Erase mode: from the cursor to the end.
const ERASE_TO_END: u16 = 0;
/// Erase mode: from the start to the cursor.
const ERASE_TO_CURSOR: u16 = 1;
/// Erase mode: the whole line.
const ERASE_LINE: u16 = 2;
/// The final byte of cursor to column (CHA).
const CURSOR_TO_COLUMN: char = 'G';
/// The final byte of character position absolute (HPA), which CHA repeats.
const COLUMN_ABSOLUTE: char = '`';
/// The final byte of cursor forward (CUF).
const CURSOR_FORWARD: char = 'C';
/// The final byte of cursor back (CUB).
const CURSOR_BACK: char = 'D';
/// The final byte of cursor up (CUU).
const CURSOR_UP: char = 'A';
/// The final byte of cursor down (CUD).
const CURSOR_DOWN: char = 'B';
/// The final byte of cursor next line (CNL): down, to the first column.
const CURSOR_NEXT_LINE: char = 'E';
/// The final byte of cursor previous line (CPL): up, to the first column.
const CURSOR_PREVIOUS_LINE: char = 'F';
/// The final byte of cursor position (CUP).
const CURSOR_POSITION: char = 'H';
/// The final byte of horizontal and vertical position (HVP), which CUP
/// repeats.
const POSITION_ABSOLUTE: char = 'f';
/// The final byte of line position absolute (VPA).
const LINE_ABSOLUTE: char = 'd';
/// The final byte of save cursor (SCOSC) as a control sequence.
const SAVE_CURSOR: char = 's';
/// The final byte of restore cursor (SCORC) as a control sequence.
const RESTORE_CURSOR: char = 'u';
/// The escape that saves the cursor (DECSC).
const SAVE_CURSOR_ESCAPE: u8 = b'7';
/// The escape that restores the cursor (DECRC).
const RESTORE_CURSOR_ESCAPE: u8 = b'8';
/// The first column or row, as control sequences number them, and the count
/// a move leaves out.
const FIRST: u16 = 1;
/// The furthest column a sequence may move to, so one cannot make a line
/// allocate a huge run of blanks.
const MAX_COLUMN: usize = 1024;
/// Most characters a line holds before it is cut into a line of its own, so
/// output with no line feeds never grows one line without bound.
pub const MAX_LINE_CELLS: usize = 64 * 1024;
/// Most rows a cursor-up is taken to climb, about the height of a tall
/// screen; the blank lines it redraws are dropped until it is back down.
const MAX_RISE: usize = 256;

/// The cursor as a save leaves it: column, pen and how far above the
/// newest line it stands.
type SavedCursor = (usize, Pen, usize);

/// The line being drawn and the lines finished so far.
///
/// A line ends at a line feed. It is kept when anything but whitespace is
/// left on it, or when it is a plain blank line; a blank line that only
/// redrew the screen is dropped: one with a cursor move, erase or other
/// sequence on it, one the cursor climbed back over, or the rest of a line
/// already shown in part. Cursor moves up and down are not applied, as a
/// log reader has already shown the lines they reach. What is drawn after a
/// saved cursor jumps to an absolute position, up to its restore or the next
/// line feed, and everything drawn on the alternate screen, is a screen
/// overlay such as a status footer, and is dropped.
#[derive(Default)]
pub struct LineDrawing {
    /// The characters of the line being drawn.
    cells: Vec<LineCell>,
    /// The column the next character lands in.
    cursor: usize,
    /// The pen the next character is drawn in.
    pen: Pen,
    /// The cursor a save kept, until it is restored.
    saved: Option<SavedCursor>,
    /// Whether a saved cursor jumped away, so what is drawn is an overlay.
    detached: bool,
    /// Whether the alternate screen is up.
    alternate: bool,
    /// Whether a sequence other than SGR reached the line.
    steered: bool,
    /// How many rows above the newest line the cursor stands.
    risen: usize,
    /// Whether the line continues one already shown in part.
    resumed: bool,
    /// Lines finished and not yet taken.
    finished: Vec<LogLine>,
}

impl LineDrawing {
    /// The lines finished since the last call.
    pub fn take_lines(&mut self) -> Vec<LogLine> {
        mem::take(&mut self.finished)
    }

    /// The line being drawn, unless nothing but whitespace is on it. It is
    /// shown now, so what follows on it starts a line of its own and its
    /// line feed alone adds no blank line.
    pub fn take_partial(&mut self) -> Option<LogLine> {
        let line = LogLine::builder().cells(self.cells.clone()).build();
        if line.is_blank() {
            return None;
        }
        self.cells.clear();
        self.cursor = 0;
        self.steered = false;
        self.resumed = true;
        Some(line)
    }

    /// Whether what is drawn lands on the line rather than an overlay.
    fn on_line(&self) -> bool {
        !self.detached && !self.alternate
    }

    /// Puts `character` at the cursor in the current pen, overwriting what
    /// is there and padding with blanks when the cursor was moved past the
    /// end; a full line is cut first.
    fn put(&mut self, character: char) {
        if self.cursor >= MAX_LINE_CELLS {
            self.cut();
        }
        let cell = LineCell::builder()
            .character(character)
            .pen(self.pen.clone())
            .build();
        if self.cursor >= self.cells.len() {
            self.cells.resize(self.cursor, blank());
            self.cells.push(cell);
        } else {
            self.cells[self.cursor] = cell;
        }
        self.cursor += 1;
    }

    /// Applies an erase in `mode`; erasing to the cursor includes the
    /// cursor's own column, as a terminal does.
    fn erase(&mut self, mode: u16) {
        let length = self.cells.len();
        match mode {
            ERASE_TO_END => self.cells.truncate(self.cursor.min(length)),
            ERASE_TO_CURSOR => self.cells[..(self.cursor + 1).min(length)].fill(blank()),
            ERASE_LINE => self.cells.clear(),
            _ => {},
        }
    }

    /// Ends the line at a line feed, keeping it unless it only redrew the
    /// screen. A line feed in an overlay ends a jump away instead, and drops
    /// the save it never restored.
    fn end_line(&mut self) {
        if self.alternate {
            return;
        }
        if self.detached {
            self.detached = false;
            self.saved = None;
            return;
        }
        let line = LogLine::builder().cells(mem::take(&mut self.cells)).build();
        let redraw = self.steered || self.resumed || self.risen > 0;
        if !line.is_blank() || !redraw {
            self.finished.push(line);
        }
        self.cursor = 0;
        self.steered = false;
        self.resumed = false;
        self.risen = self.risen.saturating_sub(1);
    }

    /// Finishes the line being drawn as it stands because it is full, and
    /// carries on drawing at the start of the next.
    fn cut(&mut self) {
        let line = LogLine::builder().cells(mem::take(&mut self.cells)).build();
        self.finished.push(line);
        self.cursor = 0;
    }

    /// Keeps the cursor for a later restore.
    fn save(&mut self) {
        self.saved = Some((self.cursor, self.pen.clone(), self.risen));
    }

    /// Puts the cursor back where a save kept it, ending any jump away.
    fn restore(&mut self) {
        if let Some((cursor, pen, risen)) = self.saved.take() {
            self.cursor = cursor;
            self.pen = pen;
            self.risen = risen;
        }
        self.detached = false;
    }

    /// Moves the cursor to `column`, numbered from one.
    fn move_to_column(&mut self, column: u16) {
        self.cursor = usize::from(column.max(FIRST)).min(MAX_COLUMN) - usize::from(FIRST);
    }

    /// Applies a control sequence other than SGR that ends in `action`.
    fn steer(&mut self, params: &Params, action: char) {
        let first = param_values(params).next();
        let count = usize::from(first.unwrap_or(FIRST).max(FIRST));
        match action {
            SAVE_CURSOR if params.is_empty() || first == Some(0) => self.save(),
            RESTORE_CURSOR => self.restore(),
            _ if !self.on_line() => {},
            ERASE_IN_LINE | ERASE_IN_DISPLAY => self.erase(first.unwrap_or(ERASE_TO_END)),
            CURSOR_TO_COLUMN | COLUMN_ABSOLUTE => self.move_to_column(first.unwrap_or(FIRST)),
            CURSOR_FORWARD => self.cursor = (self.cursor + count).min(MAX_COLUMN),
            CURSOR_BACK => self.cursor = self.cursor.saturating_sub(count),
            CURSOR_UP => self.risen = (self.risen + count).min(MAX_RISE),
            CURSOR_DOWN => self.risen = self.risen.saturating_sub(count),
            CURSOR_PREVIOUS_LINE => {
                self.risen = (self.risen + count).min(MAX_RISE);
                self.cursor = 0;
            },
            CURSOR_NEXT_LINE => {
                self.risen = self.risen.saturating_sub(count);
                self.cursor = 0;
            },
            CURSOR_POSITION | POSITION_ABSOLUTE | LINE_ABSOLUTE if self.saved.is_some() => {
                self.detached = true;
            },
            CURSOR_POSITION | POSITION_ABSOLUTE => {
                self.move_to_column(param_values(params).nth(1).unwrap_or(FIRST));
            },
            _ => {},
        }
    }
}

impl Perform for LineDrawing {
    /// Draws a character on the line.
    fn print(&mut self, character: char) {
        if self.on_line() {
            self.put(character);
        }
    }

    /// Applies the control characters that shape a line; others are dropped.
    fn execute(&mut self, byte: u8) {
        if byte == LINE_FEED {
            self.end_line();
            return;
        }
        if !self.on_line() {
            return;
        }
        match byte {
            CARRIAGE_RETURN => self.cursor = 0,
            BACKSPACE => self.cursor = self.cursor.saturating_sub(1),
            TAB => self.put(char::from(TAB)),
            _ => {},
        }
    }

    /// Sets the pen, notes the alternate screen, and applies the moves and
    /// erases a line is redrawn with; every other sequence is dropped.
    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        if !ignore && intermediates.is_empty() && action == SGR {
            self.pen.apply(params);
            return;
        }
        self.steered = true;
        if ignore {
            return;
        }
        match intermediates {
            [] => self.steer(params, action),
            [PRIVATE_MARKER]
                if matches!(action, SET_MODE_FINAL | RESET_MODE_FINAL)
                    && param_values(params).any(|mode| ALT_SCREEN_MODES.contains(&mode)) =>
            {
                self.alternate = action == SET_MODE_FINAL;
            },
            _ => {},
        }
    }

    /// Saves or restores the cursor; every other escape is dropped.
    fn esc_dispatch(&mut self, intermediates: &[u8], ignore: bool, byte: u8) {
        self.steered = true;
        if ignore || !intermediates.is_empty() {
            return;
        }
        match byte {
            SAVE_CURSOR_ESCAPE => self.save(),
            RESTORE_CURSOR_ESCAPE => self.restore(),
            _ => {},
        }
    }

    /// Drops an OSC string, such as a title.
    fn osc_dispatch(&mut self, _params: &[&[u8]], _bell_terminated: bool) {
        self.steered = true;
    }

    /// Drops a DCS string.
    fn hook(&mut self, _params: &Params, _intermediates: &[u8], _ignore: bool, _action: char) {
        self.steered = true;
    }
}

/// An erased column.
fn blank() -> LineCell {
    LineCell::builder()
        .character(BLANK)
        .pen(Pen::default())
        .build()
}
