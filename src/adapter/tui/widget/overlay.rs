use ratatui::{
    Frame,
    layout::{Position, Rect},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, BorderType, Borders},
};

/// Border and accent color, cohesive with the focused-pane border.
pub const ACCENT_COLOR: Color = Color::Cyan;
/// Drop-shadow color painted behind an overlay for depth.
const SHADOW_COLOR: Color = Color::Black;
/// Minimum overlay width, in columns.
pub const MIN_WIDTH: u16 = 44;
/// Maximum overlay width, in columns.
pub const MAX_WIDTH: u16 = 72;
/// Inner horizontal breathing room added to the content width.
pub const HORIZONTAL_PADDING: u16 = 3;
/// Borders consume the top and bottom rows of an overlay.
pub const BORDERS: u16 = 2;

/// A rounded, cyan-bordered panel titled with `title`.
pub fn panel(title: Line<'static>) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(ACCENT_COLOR))
        .title(title)
}

/// A content width clamped to the overlay bounds and the available `area`.
pub fn clamp_width(content: u16, area: Rect) -> u16 {
    content
        .saturating_add(HORIZONTAL_PADDING)
        .clamp(MIN_WIDTH, MAX_WIDTH)
        .min(area.width)
}

/// Centers a `width` x `height` rectangle within `area`.
pub fn centered(width: u16, height: u16, area: Rect) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect {
        x,
        y,
        width,
        height,
    }
}

/// Dims every cell in `area` so an overlay reads as the foreground layer.
pub fn dim_backdrop(frame: &mut Frame, area: Rect) {
    let buffer = frame.buffer_mut();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if let Some(cell) = buffer.cell_mut(Position::new(x, y)) {
                cell.set_style(Style::default().add_modifier(Modifier::DIM));
            }
        }
    }
}

/// Paints a one-cell drop shadow below and right of `modal` for depth.
pub fn draw_shadow(frame: &mut Frame, modal: Rect, area: Rect) {
    let shadow = Rect {
        x: modal.x.saturating_add(1),
        y: modal.y.saturating_add(1),
        width: modal.width,
        height: modal.height,
    }
    .intersection(area);
    let buffer = frame.buffer_mut();
    for y in shadow.top()..shadow.bottom() {
        for x in shadow.left()..shadow.right() {
            if let Some(cell) = buffer.cell_mut(Position::new(x, y)) {
                cell.set_style(Style::default().bg(SHADOW_COLOR));
            }
        }
    }
}
