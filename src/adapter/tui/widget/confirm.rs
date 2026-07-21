use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};

use super::overlay;

/// Message color.
const MESSAGE_COLOR: Color = Color::White;
/// Keyboard-hint color.
const HINT_COLOR: Color = Color::DarkGray;
/// Blank rows above the message, inside the border.
const TOP_PAD: u16 = 1;
/// Blank rows between the message and the hints.
const GAP: u16 = 1;

/// Draws a yes/no confirmation overlay: `title` on the border, `message` in the
/// body, and a `y`/`n` footer whose accept verb is `verb`, over a dimmed
/// backdrop with a soft drop shadow.
pub fn render(frame: &mut Frame, area: Rect, title: &str, message: &str, verb: &str) {
    let body = Line::from(Span::styled(
        format!("  {message}"),
        Style::default().fg(MESSAGE_COLOR),
    ));
    let footer = Line::from(Span::styled(
        format!(" y / ⏎ {verb}    n / esc cancel"),
        Style::default().fg(HINT_COLOR),
    ));
    let title = Line::from(vec![
        Span::raw(" "),
        Span::styled(
            title.to_string(),
            Style::default()
                .fg(overlay::ACCENT_COLOR)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ]);

    let content = [body.width(), footer.width(), title.width()]
        .into_iter()
        .max()
        .unwrap_or(0) as u16;
    let width = overlay::clamp_width(content, area);
    let height = (TOP_PAD + 1 + GAP + 1 + overlay::BORDERS).min(area.height);
    let modal = overlay::centered(width, height, area);

    overlay::dim_backdrop(frame, area);
    overlay::draw_shadow(frame, modal, area);
    frame.render_widget(Clear, modal);
    let block = overlay::panel(title);
    let inner = block.inner(modal);
    frame.render_widget(block, modal);

    let regions = Layout::vertical([
        Constraint::Length(TOP_PAD),
        Constraint::Length(1),
        Constraint::Length(GAP),
        Constraint::Length(1),
    ])
    .split(inner);
    frame.render_widget(Paragraph::new(body), regions[1]);
    frame.render_widget(Paragraph::new(footer), regions[3]);
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;

    #[test]
    fn renders_the_confirmation() {
        let mut terminal = Terminal::new(TestBackend::new(60, 10)).unwrap();
        terminal
            .draw(|frame| {
                render(
                    frame,
                    frame.area(),
                    "Overwrite?",
                    "A muster.yml already exists in that folder.",
                    "overwrite",
                )
            })
            .unwrap();
        insta::assert_snapshot!(terminal.backend());
    }
}
