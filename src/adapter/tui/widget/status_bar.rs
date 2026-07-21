use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

/// Leader key chord shown before the terminal-mode hints.
const LEADER_LABEL: &str = " C-a ";
/// The always-present hint pointing at the full keymap overlay.
const HELP_HINT: (&str, &str) = ("?", "help");
/// Slim hints for a process selected in the active project.
const PROCESS_HINTS: &[(&str, &str)] = &[
    ("↑↓", "move"),
    ("⏎", "attach"),
    ("s", "start/stop"),
    ("t", "autostart"),
];
/// Slim hints for a collapsed other-project row.
const PROJECT_HINTS: &[(&str, &str)] = &[("↑↓", "move"), ("→", "open"), ("d", "remove")];
/// Slim hints for an active project that has no processes yet.
const EMPTY_HINTS: &[(&str, &str)] = &[("a", "add"), ("n", "new"), ("o", "projects")];
/// Slim hints for an attached terminal; each key follows the leader chord.
const TERMINAL_HINTS: &[(&str, &str)] = &[("h", "detach"), ("s", "start/stop"), ("r", "restart")];

/// The sidebar/terminal context the status bar advertises hints for. The full
/// keymap lives in the `?` overlay; each context shows only its slim subset.
pub enum StatusContext {
    /// A process in the active project is selected.
    Process,
    /// A collapsed other-project row is selected.
    Project,
    /// The active project has no processes.
    Empty,
    /// A terminal pane is attached.
    Terminal,
}

impl StatusContext {
    /// This context's slim hint set, excluding the always-present help hint.
    fn hints(&self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::Process => PROCESS_HINTS,
            Self::Project => PROJECT_HINTS,
            Self::Empty => EMPTY_HINTS,
            Self::Terminal => TERMINAL_HINTS,
        }
    }
}

/// Color of key chords.
const KEY_COLOR: Color = Color::Cyan;
/// Color of descriptive labels.
const LABEL_COLOR: Color = Color::DarkGray;
/// Trailing spaces after each hint label for separation.
const HINT_GAP: &str = "   ";
/// Color of the crashed-process alert.
const ALERT_COLOR: Color = Color::Red;
/// Glyph preceding the crashed-process count.
const ALERT_GLYPH: &str = "⚠";

/// Prefix on a transient notice line.
const NOTICE_PREFIX: &str = " ! ";

/// Renders the status bar: a transient notice if one is set, otherwise the slim
/// hints for `context` (always ending in `? help`) plus a right-aligned alert
/// when any process has crashed so a failure is visible from any pane.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    context: StatusContext,
    crashed: usize,
    notice: Option<&str>,
) {
    if let Some(notice) = notice {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!("{NOTICE_PREFIX}{notice}"),
                Style::default()
                    .fg(ALERT_COLOR)
                    .add_modifier(Modifier::BOLD),
            ))),
            area,
        );
        return;
    }
    let mut spans = Vec::new();
    if matches!(context, StatusContext::Terminal) {
        spans.push(Span::styled(LEADER_LABEL, Style::default().fg(KEY_COLOR)));
    }
    spans.extend(hint_spans(context.hints()));
    spans.extend(hint_spans(&[HELP_HINT]));
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
    if crashed > 0 {
        render_alert(frame, area, crashed);
    }
}

/// Draws the crashed-process count pinned to the right of the status row.
fn render_alert(frame: &mut Frame, area: Rect, crashed: usize) {
    let label = format!("{ALERT_GLYPH} {crashed} crashed ");
    let width = label.chars().count() as u16;
    if area.width <= width {
        return;
    }
    let alert = Rect {
        x: area.x + area.width - width,
        y: area.y,
        width,
        height: area.height,
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            label,
            Style::default()
                .fg(ALERT_COLOR)
                .add_modifier(Modifier::BOLD),
        ))),
        alert,
    );
}

/// Builds the styled key/label spans for a set of hints.
fn hint_spans(hints: &[(&str, &str)]) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (key, label) in hints {
        spans.push(Span::styled(
            format!("{key} "),
            Style::default().fg(KEY_COLOR),
        ));
        spans.push(Span::styled(
            format!("{label}{HINT_GAP}"),
            Style::default().fg(LABEL_COLOR),
        ));
    }
    spans
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;

    #[test]
    fn shows_a_crashed_alert_pinned_right() {
        let mut terminal = Terminal::new(TestBackend::new(60, 1)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), StatusContext::Process, 2, None))
            .unwrap();
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn no_alert_when_nothing_has_crashed() {
        let mut terminal = Terminal::new(TestBackend::new(60, 1)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), StatusContext::Process, 0, None))
            .unwrap();
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn a_project_row_shows_its_own_hints() {
        let mut terminal = Terminal::new(TestBackend::new(60, 1)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), StatusContext::Project, 0, None))
            .unwrap();
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn a_notice_replaces_the_hints() {
        let mut terminal = Terminal::new(TestBackend::new(60, 1)).unwrap();
        terminal
            .draw(|frame| {
                render(
                    frame,
                    frame.area(),
                    StatusContext::Process,
                    0,
                    Some("one: no such file"),
                )
            })
            .unwrap();
        insta::assert_snapshot!(terminal.backend());
    }
}
