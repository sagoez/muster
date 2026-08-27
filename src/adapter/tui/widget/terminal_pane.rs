use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    widgets::{Block, Borders},
};

use super::theme;
use crate::adapter::tui::{emulator::TerminalEmulator, selection::GridCell};

/// Glyph drawn over the cursor cell of a focused pane, matching a solid block
/// cursor.
const CURSOR_GLYPH: char = '\u{2588}';

/// Renders the focused pane's terminal screen inside a titled border, laying an
/// active drag selection over the drawn cells in `selection_style`. When no
/// emulator is available (no processes yet), just the bordered frame is drawn.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    emulator: Option<&TerminalEmulator>,
    focused: bool,
    selection: Option<(GridCell, GridCell)>,
    selection_style: Style,
) {
    let block = Block::default()
        .title(format!(" {title} "))
        .borders(Borders::ALL)
        .border_type(theme::border_type(focused))
        .border_style(theme::border_style(focused));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if let Some(emulator) = emulator {
        draw_screen(frame, inner, emulator, focused);
    }
    if let Some(span) = selection {
        highlight(frame, inner, span, selection_style);
    }
}

/// Paints every visible grid cell of `emulator` into `inner`, then overlays the
/// cursor as a solid block when the pane is focused. Wide-character spacers are
/// emptied so the preceding wide glyph keeps both columns.
fn draw_screen(frame: &mut Frame, inner: Rect, emulator: &TerminalEmulator, focused: bool) {
    let (rows, cols) = emulator.size();
    let rows = rows.min(inner.height);
    let cols = cols.min(inner.width);
    let buffer = frame.buffer_mut();
    for row in 0..rows {
        for col in 0..cols {
            let Some(target) = buffer.cell_mut((inner.x + col, inner.y + row)) else {
                continue;
            };
            let rendered = emulator.cell(row, col);
            if rendered.is_spacer() {
                target.set_symbol("");
            } else {
                target.set_char(rendered.ch());
            }
            target.set_style(rendered.style());
        }
    }
    if focused
        && let Some((row, col)) = emulator.cursor()
        && row < rows
        && col < cols
        && let Some(target) = buffer.cell_mut((inner.x + col, inner.y + row))
    {
        target.set_char(CURSOR_GLYPH);
    }
}

/// Applies the selection style to the linear span between two grid cells,
/// clamped to the drawn pane interior.
fn highlight(frame: &mut Frame, inner: Rect, (start, end): (GridCell, GridCell), style: Style) {
    let Some(max_column) = inner.width.checked_sub(1) else {
        return;
    };
    for row in start.row()..=end.row() {
        if row >= inner.height {
            break;
        }
        let first = if row == start.row() {
            start.column()
        } else {
            0
        };
        if first > max_column {
            continue;
        }
        let last = if row == end.row() {
            end.column().min(max_column)
        } else {
            max_column
        };
        let segment = Rect::new(inner.x + first, inner.y + row, last - first + 1, 1);
        frame.buffer_mut().set_style(segment, style);
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend, style::Modifier};

    use super::*;

    /// Grid rows of the emulator backing the render.
    const ROWS: u16 = 4;
    /// Grid columns of the emulator backing the render.
    const COLS: u16 = 20;
    /// Border offset between the pane area and its interior.
    const BORDER: u16 = 1;

    fn cell(row: u16, column: u16) -> GridCell {
        GridCell::builder().row(row).column(column).build()
    }

    /// An emulator sized for the tests with `bytes` already processed.
    fn emulator(bytes: &[u8]) -> TerminalEmulator {
        let mut emulator = TerminalEmulator::new(ROWS, COLS, 0);
        emulator.process(bytes);
        emulator
    }

    /// Whether the buffer cell at pane-grid coordinates is drawn reversed.
    fn reversed(terminal: &Terminal<TestBackend>, row: u16, column: u16) -> bool {
        let cell = terminal
            .backend()
            .buffer()
            .cell((column + BORDER, row + BORDER))
            .expect("cell inside the test buffer");
        cell.style().add_modifier.contains(Modifier::REVERSED)
    }

    /// The symbol drawn at the cursor cell after feeding `hi`, for a focus state.
    fn cursor_symbol(focused: bool) -> String {
        let emulator = emulator(b"hi");
        let backend = TestBackend::new(COLS + BORDER * 2, ROWS + BORDER * 2);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|frame| {
                render(
                    frame,
                    frame.area(),
                    "pane",
                    Some(&emulator),
                    focused,
                    None,
                    theme::selection_style(),
                );
            })
            .expect("draw");
        terminal
            .backend()
            .buffer()
            .cell((2 + BORDER, BORDER))
            .expect("cursor cell inside the buffer")
            .symbol()
            .to_string()
    }

    /// The pane cursor is drawn only when the pane is focused; an unfocused pane
    /// leaves the cursor cell blank so it never looks typeable.
    #[test]
    fn the_cursor_appears_only_when_the_pane_is_focused() {
        assert_eq!(cursor_symbol(true), "\u{2588}");
        assert_eq!(cursor_symbol(false), " ");
    }

    /// The drag span is reversed; everything outside it stays untouched.
    #[test]
    fn highlights_only_the_selected_span() {
        let emulator = emulator(b"alpha beta\r\ngamma delta");
        let backend = TestBackend::new(COLS + BORDER * 2, ROWS + BORDER * 2);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|frame| {
                render(
                    frame,
                    frame.area(),
                    "pane",
                    Some(&emulator),
                    true,
                    Some((cell(0, 6), cell(1, 4))),
                    theme::selection_style(),
                );
            })
            .expect("draw");

        assert!(reversed(&terminal, 0, 6));
        assert!(reversed(&terminal, 0, COLS - 1));
        assert!(reversed(&terminal, 1, 0));
        assert!(reversed(&terminal, 1, 4));
        assert!(!reversed(&terminal, 0, 5));
        assert!(!reversed(&terminal, 1, 5));
        assert!(!reversed(&terminal, 2, 0));
        let border_cell = terminal
            .backend()
            .buffer()
            .cell((0, 0))
            .expect("border cell exists");
        assert!(
            !border_cell
                .style()
                .add_modifier
                .contains(Modifier::REVERSED)
        );
    }
}
