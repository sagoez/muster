//! A copy of one screen's cells and cursor, and the escape sequences that paint
//! it onto a freshly reset terminal.

use alacritty_terminal::vte::ansi::StandardCharset;
use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use super::{
    cursor_state::CursorState,
    emulator::{CellColor, CellFlags, CellStyle},
};

/// DECSC: saves the cursor's position, pen and charsets.
const SAVE_CURSOR: &str = "\x1b7";
/// Introduces a G0 charset designation.
const DESIGNATE_G0: &str = "\x1b(";
/// Introduces a G1 charset designation.
const DESIGNATE_G1: &str = "\x1b)";
/// Final byte designating US ASCII.
const CHARSET_ASCII: char = 'B';
/// Final byte designating the DEC special character and line drawing set.
const CHARSET_LINE_DRAWING: char = '0';

/// SGR reset, emitted before each styled run and after each row.
pub const SGR_RESET: &str = "\x1b[0m";
/// SGR parameter: bold.
const SGR_BOLD: &str = "1";
/// SGR parameter: dim.
const SGR_DIM: &str = "2";
/// SGR parameter: italic.
const SGR_ITALIC: &str = "3";
/// SGR parameter: underline.
const SGR_UNDERLINE: &str = "4";
/// SGR parameter: inverse video.
const SGR_INVERSE: &str = "7";
/// SGR parameter: hidden.
const SGR_HIDDEN: &str = "8";
/// SGR parameter: strikeout.
const SGR_STRIKEOUT: &str = "9";
/// SGR parameter that resets every attribute.
const SGR_RESET_PARAM: u8 = 0;
/// SGR sub-parameter introducing a 256-color palette index.
const SGR_COLOR_INDEXED: u8 = 5;
/// SGR sub-parameter introducing a true-color triple.
const SGR_COLOR_RGB: u8 = 2;
/// SGR parameter prefix selecting a foreground color.
const SGR_FOREGROUND: u8 = 38;
/// SGR parameter prefix selecting a background color.
const SGR_BACKGROUND: u8 = 48;

/// Control Sequence Introducer.
pub const CSI: &str = "\x1b[";
/// Final character of the select-graphic-rendition (SGR) sequence.
const SGR_FINAL: char = 'm';
/// Final character of the cursor-position (CUP) sequence.
const CUP_FINAL: char = 'H';

/// Builds the CUP sequence placing the cursor at one-based `row` and `col`.
pub fn cursor_position(row: u16, col: u16) -> String {
    format!("{CSI}{row};{col}{CUP_FINAL}")
}

/// One captured cell.
#[derive(Debug, Clone, Getters, CopyGetters, TypedBuilder)]
pub struct SnapshotCell {
    /// The base glyph.
    #[getset(get_copy = "pub")]
    ch: char,
    /// Combining characters attached to the glyph.
    #[getset(get = "pub")]
    zerowidth: Vec<char>,
    /// Resolved colors and attributes.
    #[getset(get_copy = "pub")]
    style: CellStyle,
    /// Whether this is the trailing half of a wide character.
    #[getset(get_copy = "pub")]
    spacer: bool,
}

impl SnapshotCell {
    /// Whether the cell is an unstyled blank a reset screen already shows.
    fn is_blank_default(&self) -> bool {
        !self.spacer
            && self.ch == ' '
            && self.zerowidth.is_empty()
            && self.style == CellStyle::default()
    }
}

/// A captured screen: its rows of cells, its cursor and its saved cursor.
#[derive(Debug, Clone, Getters, CopyGetters, TypedBuilder)]
pub struct ScreenSnapshot {
    /// Cells by row, then column.
    #[getset(get = "pub")]
    rows: Vec<Vec<SnapshotCell>>,
    /// The live cursor.
    #[getset(get_copy = "pub")]
    cursor: CursorState,
    /// The cursor DECSC saved.
    #[getset(get_copy = "pub")]
    saved: CursorState,
}

impl ScreenSnapshot {
    /// Appends the sequences that rebuild the saved cursor with DECSC, then
    /// put the charsets and pen back to the defaults the cells paint with.
    pub fn save_cursor(&self, rows: u16, cols: u16, out: &mut String) {
        let (row, col) = self.saved.position();
        out.push_str(&cursor_position(
            row.min(rows.saturating_sub(1)) + 1,
            col.min(cols.saturating_sub(1)) + 1,
        ));
        push_pen(self.saved, out);
        out.push_str(SAVE_CURSOR);
        push_charsets([StandardCharset::Ascii; 2], out);
        out.push_str(SGR_RESET);
    }

    /// Appends the sequences that leave the cursor at its captured position,
    /// addressed from row `origin_top` when origin mode is on, with a pending
    /// wrap restored by printing the line's last character again: a cursor
    /// position alone cannot express one.
    pub fn place_cursor(&self, rows: u16, cols: u16, origin_top: u16, out: &mut String) {
        let (row, col) = self.cursor.position();
        let row = row.min(rows.saturating_sub(1));
        let addressed = row.saturating_sub(origin_top) + 1;
        let last = cols.saturating_sub(1);
        if self.cursor.wrap_pending()
            && col >= last
            && let Some((column, cell)) = self.last_printed(row, last)
        {
            out.push_str(&cursor_position(addressed, column + 1));
            out.push_str(&sgr_sequence(cell.style()));
            out.push(cell.ch());
            out.extend(cell.zerowidth());
            out.push_str(SGR_RESET);
            return;
        }
        out.push_str(&cursor_position(addressed, col.min(last) + 1));
    }

    /// Appends the live cursor's charsets and pen, which must follow every
    /// printed character so neither changes what the repaint draws.
    pub fn restore_pen(&self, out: &mut String) {
        push_pen(self.cursor, out);
    }

    /// The cell that ends `row` at column `last` and the column it starts
    /// at: the cell itself, or the wide character a trailing spacer belongs
    /// to.
    fn last_printed(&self, row: u16, last: u16) -> Option<(u16, &SnapshotCell)> {
        let cells = self.rows.get(usize::from(row))?;
        let cell = cells.get(usize::from(last))?;
        if !cell.spacer() {
            return Some((last, cell));
        }
        let lead = last.checked_sub(1)?;
        cells.get(usize::from(lead)).map(|cell| (lead, cell))
    }

    /// Appends the sequences that paint the first `rows` by `cols` cells.
    pub fn paint_cells(&self, rows: u16, cols: u16, out: &mut String) {
        for (row, cells) in (1..=rows).zip(&self.rows) {
            let cells = &cells[..cells.len().min(usize::from(cols))];
            out.push_str(&cursor_position(row, 1));
            let used = cells
                .iter()
                .rposition(|cell| !cell.is_blank_default())
                .map_or(0, |last| last + 1);
            let mut pen = None;
            for cell in cells[..used].iter().filter(|cell| !cell.spacer()) {
                if pen != Some(cell.style()) {
                    out.push_str(&sgr_sequence(cell.style()));
                    pen = Some(cell.style());
                }
                out.push(cell.ch());
                out.extend(cell.zerowidth());
            }
            out.push_str(SGR_RESET);
        }
    }
}

/// Appends the designations of `cursor`'s charsets, then its pen.
fn push_pen(cursor: CursorState, out: &mut String) {
    push_charsets(cursor.charsets(), out);
    out.push_str(&sgr_sequence(cursor.pen()));
}

/// Appends the designations of the G0 then G1 `charsets`.
pub fn push_charsets(charsets: [StandardCharset; 2], out: &mut String) {
    for (designate, charset) in [DESIGNATE_G0, DESIGNATE_G1].into_iter().zip(charsets) {
        out.push_str(designate);
        out.push(match charset {
            StandardCharset::Ascii => CHARSET_ASCII,
            StandardCharset::SpecialCharacterAndLineDrawing => CHARSET_LINE_DRAWING,
        });
    }
}

/// The SGR sequence that sets exactly `style` from a reset pen.
fn sgr_sequence(style: CellStyle) -> String {
    let flags: CellFlags = style.flags();
    let mut params = vec![SGR_RESET_PARAM.to_string()];
    let attributes = [
        (flags.bold(), SGR_BOLD),
        (flags.dim(), SGR_DIM),
        (flags.italic(), SGR_ITALIC),
        (flags.underline(), SGR_UNDERLINE),
        (flags.inverse(), SGR_INVERSE),
        (flags.hidden(), SGR_HIDDEN),
        (flags.strikeout(), SGR_STRIKEOUT),
    ];
    params.extend(
        attributes
            .into_iter()
            .filter(|(on, _)| *on)
            .map(|(_, code)| code.to_string()),
    );
    params.extend(color_params(SGR_FOREGROUND, style.fg()));
    params.extend(color_params(SGR_BACKGROUND, style.bg()));
    format!("{CSI}{}{SGR_FINAL}", params.join(";"))
}

/// The SGR parameters selecting `color` under `prefix` (38 or 48), or none for
/// the default color.
fn color_params(prefix: u8, color: CellColor) -> Vec<String> {
    match color {
        CellColor::Default => Vec::new(),
        CellColor::Indexed(index) => vec![
            prefix.to_string(),
            SGR_COLOR_INDEXED.to_string(),
            index.to_string(),
        ],
        CellColor::Rgb(r, g, b) => vec![
            prefix.to_string(),
            SGR_COLOR_RGB.to_string(),
            r.to_string(),
            g.to_string(),
            b.to_string(),
        ],
    }
}
