use ratatui::{
    Frame,
    layout::Rect,
    style::Style,
    widgets::{Block, Borders},
};
use tui_term::widget::PseudoTerminal;
use vt100::Screen;

use super::theme;
use crate::adapter::tui::selection::GridCell;

/// Renders the focused pane's terminal screen inside a titled border, laying an
/// active drag selection over the drawn cells in `selection_style`. When no
/// screen is available (no processes yet), just the bordered frame is drawn.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    screen: Option<&Screen>,
    focused: bool,
    selection: Option<(GridCell, GridCell)>,
    selection_style: Style,
) {
    let block = Block::default()
        .title(format!(" {title} "))
        .borders(Borders::ALL)
        .border_style(theme::border_style(focused));
    let inner = block.inner(area);
    match screen {
        Some(screen) => frame.render_widget(PseudoTerminal::new(screen).block(block), area),
        None => frame.render_widget(block, area),
    }
    if let Some(span) = selection {
        highlight(frame, inner, span, selection_style);
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
    use vt100::Parser;

    use super::*;

    /// Grid rows of the parser backing the render.
    const ROWS: u16 = 4;
    /// Grid columns of the parser backing the render.
    const COLS: u16 = 20;
    /// Border offset between the pane area and its interior.
    const BORDER: u16 = 1;

    fn cell(row: u16, column: u16) -> GridCell {
        GridCell::builder().row(row).column(column).build()
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

    /// The drag span is reversed; everything outside it stays untouched.
    #[test]
    fn highlights_only_the_selected_span() {
        let mut parser = Parser::new(ROWS, COLS, 0);
        parser.process(b"alpha beta\r\ngamma delta");
        let backend = TestBackend::new(COLS + BORDER * 2, ROWS + BORDER * 2);
        let mut terminal = Terminal::new(backend).expect("test terminal");
        terminal
            .draw(|frame| {
                render(
                    frame,
                    frame.area(),
                    "pane",
                    Some(parser.screen()),
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
