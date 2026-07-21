use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};

use super::{overlay, theme};

/// Title shown on the help overlay border.
const TITLE: &str = "Keybindings";
/// Blank rows above and below the keymap, inside the border.
const VERTICAL_PAD: u16 = 1;
/// Left indent inside the border, before the first column.
const LEFT_INDENT: u16 = 1;
/// Columns of blank space between a key chord and its description.
const KEY_GAP: usize = 2;
/// Columns of blank space between the two keymap columns.
const COLUMN_GAP: u16 = 3;
/// Indent before each key chord within a column.
const ROW_INDENT: &str = "  ";

/// A titled group of `(key, action)` rows in the keymap.
type Section = (&'static str, &'static [(&'static str, &'static str)]);

/// The keymap's left column: the sidebar keys, which act directly.
const LEFT_SECTIONS: &[Section] = &[
    ("Navigate", &[
        ("↑ / k", "up"),
        ("↓ / j", "down"),
        ("→ / l / ⏎", "open / into"),
        ("← / h", "back"),
    ]),
    ("Process", &[
        ("⏎", "attach"),
        ("s", "start / stop"),
        ("r", "restart"),
        ("p", "pause / resume"),
        ("x", "force kill"),
        ("t", "autostart"),
        ("a", "add"),
    ]),
    ("General", &[
        ("N", "notifications"),
        ("?", "help"),
        ("q", "quit"),
    ]),
];
/// The keymap's right column: the sidebar project keys, then every terminal-mode
/// command, each of which is a chord entered after the `C-a` leader.
const RIGHT_SECTIONS: &[Section] = &[
    ("Project", &[
        ("→ / ⏎", "activate"),
        ("d", "remove"),
        ("n", "new"),
        ("o", "switcher"),
    ]),
    ("Terminal (after C-a)", &[
        ("h", "detach"),
        ("↑↓ / j k", "move"),
        ("s", "start / stop"),
        ("r", "restart"),
        ("p", "pause / resume"),
        ("x", "force kill"),
        ("a", "add"),
        ("n", "new"),
        ("N", "notifications"),
        ("o", "switcher"),
        ("?", "help"),
        ("q", "quit"),
    ]),
];

/// Draws the full-keymap overlay in two columns, centered over a dimmed backdrop.
/// This is the complete reference; the status bar shows only a slim, contextual
/// subset of these bindings.
pub fn render(frame: &mut Frame, area: Rect) {
    let key_width = LEFT_SECTIONS
        .iter()
        .chain(RIGHT_SECTIONS)
        .flat_map(|(_, hints)| hints.iter())
        .map(|(key, _)| key.chars().count())
        .max()
        .unwrap_or(0);
    let left = column_lines(LEFT_SECTIONS, key_width);
    let right = column_lines(RIGHT_SECTIONS, key_width);

    let left_width = max_width(&left);
    let right_width = max_width(&right);
    let content = left_width + COLUMN_GAP + right_width + LEFT_INDENT;
    let width = overlay::clamp_width(content, area);
    let rows = left.len().max(right.len()) as u16;
    let height = (rows + VERTICAL_PAD * 2 + overlay::BORDERS).min(area.height);
    let modal = overlay::centered(width, height, area);

    overlay::dim_backdrop(frame, area);
    overlay::draw_shadow(frame, modal, area);
    frame.render_widget(Clear, modal);
    let title = Line::from(vec![
        Span::raw(" "),
        Span::styled(
            TITLE,
            Style::default()
                .fg(overlay::ACCENT_COLOR)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ]);
    let block = overlay::panel(title);
    let inner = block.inner(modal);
    frame.render_widget(block, modal);

    let body = Rect {
        x: inner.x + LEFT_INDENT,
        y: inner.y + VERTICAL_PAD,
        width: inner.width.saturating_sub(LEFT_INDENT),
        height: inner.height.saturating_sub(VERTICAL_PAD),
    };
    let columns = Layout::horizontal([
        Constraint::Length(left_width),
        Constraint::Length(COLUMN_GAP),
        Constraint::Length(right_width),
    ])
    .split(body);
    frame.render_widget(Paragraph::new(left), columns[0]);
    frame.render_widget(Paragraph::new(right), columns[2]);
}

/// Renders one column's sections into styled lines: a bold header per section,
/// then its key/action rows with keys padded to `key_width` for alignment.
fn column_lines(sections: &[Section], key_width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for (title, hints) in sections {
        lines.push(Line::from(Span::styled(
            *title,
            Style::default()
                .fg(theme::HEADER_COLOR)
                .add_modifier(Modifier::BOLD),
        )));
        for (key, label) in *hints {
            let pad = " ".repeat(key_width - key.chars().count() + KEY_GAP);
            lines.push(Line::from(vec![
                Span::raw(ROW_INDENT),
                Span::styled(*key, Style::default().fg(overlay::ACCENT_COLOR)),
                Span::raw(pad),
                Span::styled(*label, Style::default().fg(theme::SELECTED_COLOR)),
            ]));
        }
    }
    lines
}

/// The widest rendered line in `lines`, in columns.
fn max_width(lines: &[Line<'static>]) -> u16 {
    lines.iter().map(|line| line.width()).max().unwrap_or(0) as u16
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;

    #[test]
    fn renders_the_keymap_in_two_columns() {
        let mut terminal = Terminal::new(TestBackend::new(72, 24)).unwrap();
        terminal.draw(|frame| render(frame, frame.area())).unwrap();
        insta::assert_snapshot!(terminal.backend());
    }
}
