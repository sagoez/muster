//! What a process's output leaves on each line, worked out the way a log
//! reader expects rather than the way a screen redraws: one line at a time,
//! with carriage returns, backspaces, column moves and erases applied to it,
//! and everything that only redraws the screen around it dropped.
//!
//! Every step costs time in proportion to the bytes that drive it, never to
//! the length of the line it lands on: a cell is a `Copy` value, an erase to
//! the end or of the whole line only shortens the cells, and an erase to the
//! cursor is recorded as an [`Erasure`] rather than written into every cell
//! it covers. The cells are resolved against the erasures only when a line
//! is handed out.

use std::{collections::BTreeMap, mem};

use alacritty_terminal::vte::{Params, Perform};
use unicode_width::UnicodeWidthChar;

use super::{
    erasure::Erasure,
    escape::{
        ALT_SCREEN_MODES, FULL_RESET_FINAL, LINE_FEED, PRIVATE_MARKER, RESET_MODE_FINAL,
        SET_MODE_FINAL, SGR_FINAL, param_values,
    },
    line_cell::LineCell,
    log_line::LogLine,
    pen::Pen,
    saved_cursor::SavedCursor,
};
use crate::domain::value::steers_the_terminal;

/// Returns the cursor to the start of the line.
const CARRIAGE_RETURN: u8 = b'\r';
/// Moves the cursor one column left.
const BACKSPACE: u8 = b'\x08';
/// Moves to the next tab stop; kept as a tab so indentation survives.
const TAB: u8 = b'\t';
/// Columns between tab stops.
const TAB_STOP: usize = 8;
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
/// The furthest column a sequence may move the cursor to, so one cannot make
/// a line allocate a huge run of blanks.
pub const MAX_COLUMN: usize = 1024;
/// Most columns a line holds before it is cut into a line of its own, so
/// output with no line feeds never grows one line without bound.
pub const MAX_LINE_CELLS: usize = 64 * 1024;
/// Most rows a cursor-up is taken to climb, about the height of a tall
/// screen; the blank lines it redraws are dropped until it is back down.
const MAX_RISE: usize = 256;
/// Most bytes of combining marks one glyph carries; more are dropped, so a
/// stack of marks cannot grow one glyph without bound.
const MAX_MARK_BYTES: usize = 32;
/// The widest glyph a line holds: a tab spans at most one tab stop.
const MAX_GLYPH_WIDTH: usize = TAB_STOP;

/// The line being drawn and the lines finished so far.
///
/// A line ends at a line feed. It is kept when anything but whitespace is
/// left on it, or when it is a plain blank line; a blank line that only
/// redrew the screen is dropped: one with a cursor move, erase or other
/// sequence on it, or one the cursor climbed back over. Cursor moves up and
/// down are not applied, as a log reader has already shown the lines they
/// reach. What is drawn after a saved cursor jumps to an absolute position,
/// up to its restore or the next line feed, and everything drawn on the
/// alternate screen, is a screen overlay such as a status footer, and is
/// dropped. A save not restored by the end of its line is forgotten.
///
/// A glyph wider than one column takes its columns whole, and overwriting
/// any of them blanks the rest, as a terminal does; a combining mark joins
/// the glyph before the cursor. A control character never lands on a line,
/// however the parser hands it over, and a format character (a bidi control
/// or a zero-width character) lands as its escaped text.
#[derive(Default)]
pub struct LineDrawing {
    /// The columns of the line being drawn.
    cells: Vec<LineCell>,
    /// The combining marks on the glyph starting at each column.
    marks: BTreeMap<usize, String>,
    /// The column the next glyph lands in.
    cursor: usize,
    /// The pen the next glyph is drawn in.
    pen: Pen,
    /// The stamp the next cell is drawn with.
    stamp: u32,
    /// The erases to the cursor not yet applied, oldest first: their stamps
    /// rise and their extents fall.
    erasures: Vec<Erasure>,
    /// The cursor a save kept, until it is restored or its line ends.
    saved: Option<SavedCursor>,
    /// Whether a saved cursor jumped away, so what is drawn is an overlay.
    detached: bool,
    /// Whether the alternate screen is up.
    alternate: bool,
    /// Whether a sequence other than SGR reached the line.
    steered: bool,
    /// Whether the line was redrawn in place: a carriage return, backspace,
    /// erase or cursor move landed on it.
    rewritten: bool,
    /// How many rows above the newest line the cursor stands.
    risen: usize,
    /// Columns of the line already handed out by [`LineDrawing::take_partial`].
    shown: usize,
    /// Whether anything was drawn over or erased from those columns since.
    shown_changed: bool,
    /// Lines finished and not yet taken.
    finished: Vec<LogLine>,
}

impl LineDrawing {
    /// The lines finished since the last call.
    pub fn take_lines(&mut self) -> Vec<LogLine> {
        mem::take(&mut self.finished)
    }

    /// Whether the line being drawn was never redrawn in place, so what is
    /// on it now is not a passing frame of a progress bar.
    pub fn is_settled(&self) -> bool {
        !self.rewritten
    }

    /// What the line being drawn shows past what was handed out of it
    /// already, unless that is blank. It counts as shown from then on, and
    /// the line keeps being drawn: its line feed hands out only what follows,
    /// or the whole line again when what was shown changed.
    pub fn take_partial(&mut self) -> Option<LogLine> {
        let line = self.snapshot();
        let end = line.shown_end();
        if end <= self.shown {
            return None;
        }
        let part = slice(&line, self.shown, end);
        self.shown = end;
        Some(part)
    }

    /// The whole line being drawn as it stands, unless it is blank, without
    /// changing it.
    pub fn peek(&self) -> Option<LogLine> {
        let line = self.snapshot();
        (!line.is_blank()).then_some(line)
    }

    /// Ends the line being drawn where it stands, as the end of the output
    /// does, keeping it unless nothing is left of it to show.
    pub fn finish(&mut self) {
        self.close_line(false);
    }

    /// Ends the line being drawn where it stands, then forgets everything
    /// drawing it left behind, as at the start of a fresh terminal.
    pub fn reset(&mut self) {
        self.finish();
        let finished = mem::take(&mut self.finished);
        *self = Self::default();
        self.finished = finished;
    }

    /// Whether what is drawn lands on the line rather than an overlay.
    fn on_line(&self) -> bool {
        !self.detached && !self.alternate
    }

    /// Draws `character` at the cursor in the current pen, or joins it to
    /// the glyph before the cursor when it takes no column of its own.
    fn print_char(&mut self, character: char) {
        match character.width() {
            Some(0) => self.mark(character),
            Some(width) => self.draw(character, width),
            None => {},
        }
    }

    /// Puts a glyph `width` columns wide at the cursor, blanking what is left
    /// of any glyph it overwrites in part, and padding with blanks when the
    /// cursor stands past the end; a full line is cut first.
    fn draw(&mut self, character: char, width: usize) {
        if self.cursor + width > MAX_LINE_CELLS {
            self.cut();
        }
        let start = self.cursor;
        let end = start + width;
        self.pad_to(end);
        for column in start..end {
            self.break_glyph(column);
        }
        self.cells[start] = LineCell::builder()
            .character(character)
            .pen(self.pen)
            .width(u8::try_from(width).unwrap_or(u8::MAX))
            .stamp(self.stamp)
            .build();
        for column in start + 1..end {
            self.cells[column] = LineCell::continuation(self.pen, self.stamp);
        }
        self.touch(start);
        self.cursor = end;
    }

    /// Joins the combining `mark` to the glyph before the cursor, unless
    /// there is none or it already carries all the marks it may.
    fn mark(&mut self, mark: char) {
        let Some(lead) = self
            .cursor
            .checked_sub(1)
            .and_then(|column| self.lead_of(column))
        else {
            return;
        };
        if self.is_erased(lead) {
            return;
        }
        let marks = self.marks.entry(lead).or_default();
        if marks.len() + mark.len_utf8() <= MAX_MARK_BYTES {
            marks.push(mark);
        }
        self.touch(lead);
    }

    /// Moves to the next tab stop. Past the end of the line the tab is kept
    /// as a glyph spanning to the stop, so indentation survives; over drawn
    /// columns it only moves, as a terminal's does.
    fn tab(&mut self) {
        let stop = (self.cursor / TAB_STOP + 1) * TAB_STOP;
        if self.cursor >= self.cells.len() {
            self.draw(char::from(TAB), stop - self.cursor);
        } else {
            self.cursor = stop.min(MAX_LINE_CELLS);
        }
    }

    /// Where the glyph covering `column` starts, when a cell is there and
    /// belongs to one.
    fn lead_of(&self, column: usize) -> Option<usize> {
        if column >= self.cells.len() {
            return None;
        }
        let floor = column.saturating_sub(MAX_GLYPH_WIDTH);
        (floor..=column)
            .rev()
            .find(|index| !self.cells[*index].continues())
    }

    /// Readies `column` to be drawn over: the glyph covering it loses its
    /// marks, and when it is wider than one column it is blanked whole, as
    /// overwriting part of it leaves nothing of it.
    fn break_glyph(&mut self, column: usize) {
        if column >= self.cells.len() {
            return;
        }
        let Some(lead) = self.lead_of(column) else {
            self.cells[column] = LineCell::blank(self.stamp);
            return;
        };
        self.marks.remove(&lead);
        let width = usize::from(self.cells[lead].width());
        if width <= 1 {
            return;
        }
        let end = (lead + width).min(self.cells.len());
        for index in lead..end {
            self.cells[index] = LineCell::blank(self.stamp);
        }
        self.touch(lead);
    }

    /// Pads the line with blanks up to `end` columns.
    fn pad_to(&mut self, end: usize) {
        if self.cells.len() < end {
            self.cells.resize(end, LineCell::blank(self.stamp));
        }
    }

    /// Notes that `column` was drawn over or erased, which changes what was
    /// shown of the line when it lies there.
    fn touch(&mut self, column: usize) {
        if column < self.shown {
            self.shown_changed = true;
        }
    }

    /// Applies an erase in `mode`; erasing to the cursor includes the
    /// cursor's own column, as a terminal does.
    fn erase(&mut self, mode: u16) {
        self.rewritten = true;
        match mode {
            ERASE_TO_END => {
                let at = self.cursor.min(self.cells.len());
                if at < self.cells.len() {
                    self.break_glyph(at);
                    self.cells.truncate(at);
                    drop(self.marks.split_off(&at));
                    self.touch(at);
                }
            },
            ERASE_TO_CURSOR => self.erase_to_cursor(),
            ERASE_LINE => {
                self.cells.clear();
                self.marks.clear();
                self.erasures.clear();
                self.touch(0);
            },
            _ => {},
        }
    }

    /// Erases from the start of the line to the cursor by recording it: the
    /// cells it covers read as blank until drawn over again. The older
    /// erasures it reaches past are dropped, so the extents keep falling as
    /// the stamps rise, and each erase costs constant time amortized.
    fn erase_to_cursor(&mut self) {
        let extent = (self.cursor + 1).min(self.cells.len());
        if extent == 0 {
            return;
        }
        if self.stamp == u32::MAX {
            self.apply_erasures();
        }
        self.stamp += 1;
        while self
            .erasures
            .last()
            .is_some_and(|erasure| erasure.extent() <= extent)
        {
            self.erasures.pop();
        }
        self.erasures
            .push(Erasure::builder().stamp(self.stamp).extent(extent).build());
        self.touch(0);
    }

    /// Writes every recorded erasure into the cells it covers and starts the
    /// stamps over, once they run out.
    fn apply_erasures(&mut self) {
        for index in 0..self.cells.len() {
            if self.is_erased(index) {
                self.cells[index] = LineCell::blank(0);
                self.marks.remove(&index);
            }
        }
        for cell in &mut self.cells {
            *cell = cell.restamped(0);
        }
        self.erasures.clear();
        self.stamp = 0;
    }

    /// Whether an erasure recorded after the cell at `index` was drawn
    /// covers it: of those, the oldest reaches furthest.
    fn is_erased(&self, index: usize) -> bool {
        let stamp = self.cells[index].stamp();
        let newer = self
            .erasures
            .partition_point(|erasure| erasure.stamp() <= stamp);
        self.erasures
            .get(newer)
            .is_some_and(|erasure| erasure.extent() > index)
    }

    /// The line being drawn as it reads: erased columns blank, and a glyph
    /// that lost any of its columns blank whole.
    fn snapshot(&self) -> LogLine {
        let mut cells: Vec<LineCell> = if self.erasures.is_empty() {
            self.cells.clone()
        } else {
            (0..self.cells.len())
                .map(|index| {
                    if self.is_erased(index) {
                        LineCell::blank(0)
                    } else {
                        self.cells[index]
                    }
                })
                .collect()
        };
        let mut index = 0;
        while index < cells.len() {
            let cell = cells[index];
            let end = index + usize::from(cell.width());
            let whole = end <= cells.len()
                && cells[index + 1..end]
                    .iter()
                    .all(|continuation| continuation.continues());
            if cell.continues() || !whole {
                cells[index] = LineCell::blank(0);
                index += 1;
            } else {
                index = end;
            }
        }
        let marks = self
            .marks
            .iter()
            .filter(|(index, _)| cells.get(**index) == self.cells.get(**index))
            .map(|(index, marks)| (*index, marks.clone()))
            .collect();
        LogLine::builder().cells(cells).marks(marks).build()
    }

    /// Ends the line being drawn and hands out what is left to show of it:
    /// all of it, unless part was shown before, when only the rest is, or
    /// all of it again if the part shown changed since. A blank line is kept
    /// only when a line feed ended it and it did not just redraw the screen.
    fn close_line(&mut self, fed: bool) {
        let line = self.snapshot();
        let kept = if self.shown == 0 {
            let redraw = self.steered || self.risen > 0;
            (!line.is_blank() || (fed && !redraw)).then_some(line)
        } else if self.shown_changed {
            (!line.is_blank()).then_some(line)
        } else {
            let rest = slice(&line, self.shown, line.cells().len());
            (!rest.is_blank()).then_some(rest)
        };
        self.finished.extend(kept);
        self.clear_line();
    }

    /// Starts a fresh line at the first column.
    fn clear_line(&mut self) {
        self.cells.clear();
        self.marks.clear();
        self.erasures.clear();
        self.stamp = 0;
        self.cursor = 0;
        self.steered = false;
        self.rewritten = false;
        self.shown = 0;
        self.shown_changed = false;
    }

    /// Ends the line at a line feed, keeping it unless it only redrew the
    /// screen. A save not restored by now is forgotten, and a line feed in
    /// an overlay ends the jump away: the line drawn before it is done, as
    /// the cursor now stands on another row.
    fn end_line(&mut self) {
        if self.alternate {
            return;
        }
        self.close_line(true);
        if mem::take(&mut self.detached) {
            self.risen = 0;
        } else {
            self.risen = self.risen.saturating_sub(1);
        }
        self.saved = None;
    }

    /// Finishes the line being drawn as it stands because it is full, and
    /// carries on drawing at the start of the next.
    fn cut(&mut self) {
        self.close_line(false);
    }

    /// Keeps the cursor for a later restore.
    fn save(&mut self) {
        self.saved = Some(
            SavedCursor::builder()
                .column(self.cursor)
                .pen(self.pen)
                .risen(self.risen)
                .build(),
        );
    }

    /// Puts the cursor back where a save kept it, ending any jump away.
    fn restore(&mut self) {
        if let Some(saved) = self.saved.take() {
            self.cursor = saved.column();
            self.pen = saved.pen();
            self.risen = saved.risen();
            self.rewritten = true;
        }
        self.detached = false;
    }

    /// Clears everything, as a full reset (RIS) does: the line being drawn is
    /// wiped with the screen it is on, and the pen, saved cursor, overlay and
    /// alternate screen are gone.
    fn full_reset(&mut self) {
        self.clear_line();
        self.pen = Pen::default();
        self.saved = None;
        self.detached = false;
        self.alternate = false;
        self.risen = 0;
        self.steered = true;
    }

    /// Moves the cursor to `column`, numbered from one.
    fn move_to_column(&mut self, column: u16) {
        self.cursor = usize::from(column.max(FIRST)).min(MAX_COLUMN) - usize::from(FIRST);
        self.rewritten = true;
    }

    /// Moves the cursor `count` columns right, never past [`MAX_COLUMN`] and
    /// never back from a column already beyond it.
    fn forward(&mut self, count: usize) {
        if self.cursor < MAX_COLUMN {
            self.cursor = (self.cursor + count).min(MAX_COLUMN);
        }
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
            CURSOR_FORWARD => self.forward(count),
            CURSOR_BACK => {
                self.cursor = self.cursor.saturating_sub(count);
                self.rewritten = true;
            },
            CURSOR_UP | CURSOR_PREVIOUS_LINE => {
                self.risen = (self.risen + count).min(MAX_RISE);
                if action == CURSOR_PREVIOUS_LINE {
                    self.cursor = 0;
                }
                self.rewritten = true;
            },
            CURSOR_DOWN | CURSOR_NEXT_LINE => {
                self.risen = self.risen.saturating_sub(count);
                if action == CURSOR_NEXT_LINE {
                    self.cursor = 0;
                }
                self.rewritten = true;
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
    /// Draws a character on the line. A control character is dropped: one
    /// reaches here only when the parser was handed it in pieces. A format
    /// character is drawn as its escaped text, as everywhere muster shows a
    /// process's text.
    fn print(&mut self, character: char) {
        if !self.on_line() || character.is_control() {
            return;
        }
        if steers_the_terminal(character) {
            for escaped in character.escape_default() {
                self.print_char(escaped);
            }
            return;
        }
        self.print_char(character);
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
            CARRIAGE_RETURN => {
                self.cursor = 0;
                self.rewritten = true;
            },
            BACKSPACE => {
                self.cursor = self.cursor.saturating_sub(1);
                self.rewritten = true;
            },
            TAB => self.tab(),
            _ => {},
        }
    }

    /// Sets the pen, notes the alternate screen, and applies the moves and
    /// erases a line is redrawn with; every other sequence is dropped.
    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        if !ignore && intermediates.is_empty() && action == SGR_FINAL {
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

    /// Saves or restores the cursor, or clears everything at a full reset;
    /// every other escape is dropped.
    fn esc_dispatch(&mut self, intermediates: &[u8], ignore: bool, byte: u8) {
        self.steered = true;
        if ignore || !intermediates.is_empty() {
            return;
        }
        match byte {
            SAVE_CURSOR_ESCAPE => self.save(),
            RESTORE_CURSOR_ESCAPE => self.restore(),
            FULL_RESET_FINAL => self.full_reset(),
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

/// The columns of `line` from `from` up to `end`, with their marks, going on
/// from the part before `from` when that is not empty.
fn slice(line: &LogLine, from: usize, end: usize) -> LogLine {
    let end = end.min(line.cells().len());
    let from = from.min(end);
    let marks = line
        .marks()
        .range(from..end)
        .map(|(index, marks)| (index - from, marks.clone()))
        .collect();
    LogLine::builder()
        .cells(line.cells()[from..end].to_vec())
        .marks(marks)
        .continues(from > 0)
        .build()
}
