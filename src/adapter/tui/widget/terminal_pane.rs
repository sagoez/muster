use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, Borders},
};
use tui_term::widget::PseudoTerminal;
use vt100::Screen;

use super::theme;

/// Renders the focused pane's terminal screen inside a titled border. When no
/// screen is available (no processes yet), just the bordered frame is drawn.
pub fn render(frame: &mut Frame, area: Rect, title: &str, screen: Option<&Screen>, focused: bool) {
    let block = Block::default()
        .title(format!(" {title} "))
        .borders(Borders::ALL)
        .border_style(theme::border_style(focused));
    match screen {
        Some(screen) => frame.render_widget(PseudoTerminal::new(screen).block(block), area),
        None => frame.render_widget(block, area),
    }
}
