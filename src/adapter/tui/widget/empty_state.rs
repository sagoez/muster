use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::Style,
    text::{Line, Span},
    widgets::Paragraph,
};

use super::{overlay, theme};

/// Heading shown in the main pane when the workspace has no processes.
const HEADING: &str = "No processes yet";
/// Prompt lines, each an (before-key, key, after-key) triple.
const PROMPTS: &[(&str, &str, &str)] = &[
    ("press ", "a", " to add a process"),
    ("or ", "n", " to start a new project"),
];
/// Border inset, matching the surrounding terminal-pane frame.
const BORDER_INSET: u16 = 1;

/// Draws a centered hint inviting the user to add their first process, inside
/// the terminal pane's border. Skipped when the pane is too small to fit it.
pub fn render(frame: &mut Frame, area: Rect) {
    let inner = Rect {
        x: area.x + BORDER_INSET,
        y: area.y + BORDER_INSET,
        width: area.width.saturating_sub(BORDER_INSET * 2),
        height: area.height.saturating_sub(BORDER_INSET * 2),
    };
    let mut lines = vec![
        Line::from(Span::styled(
            HEADING,
            Style::default().fg(theme::HEADER_COLOR),
        )),
        Line::default(),
    ];
    lines.extend(PROMPTS.iter().map(|(before, key, after)| {
        Line::from(vec![
            Span::styled(*before, Style::default().fg(theme::DESCRIPTION_COLOR)),
            Span::styled(*key, Style::default().fg(overlay::ACCENT_COLOR)),
            Span::styled(*after, Style::default().fg(theme::DESCRIPTION_COLOR)),
        ])
    }));

    let height = lines.len() as u16;
    if inner.height < height || inner.width == 0 {
        return;
    }
    let centered = Rect {
        x: inner.x,
        y: inner.y + (inner.height - height) / 2,
        width: inner.width,
        height,
    };
    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), centered);
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;

    #[test]
    fn renders_the_add_process_hint() {
        let mut terminal = Terminal::new(TestBackend::new(48, 12)).unwrap();
        terminal.draw(|frame| render(frame, frame.area())).unwrap();
        insta::assert_snapshot!(terminal.backend());
    }
}
