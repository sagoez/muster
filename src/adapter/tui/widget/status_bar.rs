use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

/// One compact key/action hint.
type Hint = (&'static str, &'static str);

/// Leader key chord shown before the terminal-mode hints.
const LEADER_LABEL: &str = " C-a ";
/// The always-present hint pointing at the full keymap overlay.
const HELP_HINT: Hint = ("?", "help");
/// Process navigation hint.
const MOVE_HINT: Hint = ("↑↓", "move");
/// Process attachment hint.
const ATTACH_HINT: Hint = ("⏎", "attach");
/// Process start or graceful-stop hint.
const START_STOP_HINT: Hint = ("s", "start/stop");
/// Process restart hint.
const RESTART_HINT: Hint = ("r", "restart");
/// Immediate process kill hint.
const KILL_HINT: Hint = ("x", "kill");
/// Process autostart hint.
const AUTOSTART_HINT: Hint = ("t", "autostart");
/// Add-process hint.
const ADD_HINT: Hint = ("a", "add");
/// Attached terminal detach hint.
const DETACH_HINT: Hint = ("h", "detach");
/// Slim hints for a process selected in the active project.
const PROCESS_HINTS: &[Hint] = &[
    MOVE_HINT,
    ATTACH_HINT,
    START_STOP_HINT,
    RESTART_HINT,
    KILL_HINT,
    AUTOSTART_HINT,
    ADD_HINT,
];
/// Process hints in retention order when the status row is narrow.
const PROCESS_HINT_PRIORITY: &[Hint] = &[
    START_STOP_HINT,
    ADD_HINT,
    RESTART_HINT,
    KILL_HINT,
    MOVE_HINT,
    ATTACH_HINT,
    AUTOSTART_HINT,
];
/// Slim hints for a collapsed other-project row.
const PROJECT_HINTS: &[Hint] = &[("↑↓", "move"), ("→", "open"), ("d", "remove")];
/// Slim hints for an active project that has no processes yet.
const EMPTY_HINTS: &[Hint] = &[ADD_HINT, ("n", "new"), ("o", "projects")];
/// Slim hints for an attached terminal; each key follows the leader chord.
const TERMINAL_HINTS: &[Hint] = &[DETACH_HINT, START_STOP_HINT, RESTART_HINT, KILL_HINT];
/// Terminal hints in retention order when the status row is narrow.
const TERMINAL_HINT_PRIORITY: &[Hint] = &[START_STOP_HINT, RESTART_HINT, KILL_HINT, DETACH_HINT];

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
    fn hints(&self, available_width: u16) -> Vec<Hint> {
        match self {
            Self::Process => process_hints(available_width),
            Self::Project => PROJECT_HINTS.to_vec(),
            Self::Empty => EMPTY_HINTS.to_vec(),
            Self::Terminal => {
                prioritized_hints(TERMINAL_HINTS, TERMINAL_HINT_PRIORITY, available_width)
            },
        }
    }
}

/// Color of key chords.
const KEY_COLOR: Color = Color::Cyan;
/// Color of descriptive labels.
const LABEL_COLOR: Color = Color::DarkGray;
/// Background of the status row while the terminal leader chord is pending.
const LEADER_PENDING_BACKGROUND: Color = Color::Cyan;
/// Foreground of the status row while the terminal leader chord is pending.
const LEADER_PENDING_FOREGROUND: Color = Color::Black;
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
/// when any process has crashed. A pending terminal leader fills the row with a
/// high-contrast background until the next key completes the chord.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    context: StatusContext,
    crashed: usize,
    notice: Option<&str>,
    leader_pending: bool,
) {
    let leader_pending = leader_pending && matches!(context, StatusContext::Terminal);
    if let Some(notice) = notice {
        let style = if leader_pending {
            Style::default()
                .fg(LEADER_PENDING_FOREGROUND)
                .bg(LEADER_PENDING_BACKGROUND)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(ALERT_COLOR)
                .add_modifier(Modifier::BOLD)
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!("{NOTICE_PREFIX}{notice}"),
                style,
            )))
            .style(style),
            area,
        );
        return;
    }
    let key_color = if leader_pending {
        LEADER_PENDING_FOREGROUND
    } else {
        KEY_COLOR
    };
    let label_color = if leader_pending {
        LEADER_PENDING_FOREGROUND
    } else {
        LABEL_COLOR
    };
    let mut spans = Vec::new();
    if matches!(context, StatusContext::Terminal) {
        spans.push(Span::styled(LEADER_LABEL, Style::default().fg(key_color)));
    }
    let alert_width = if crashed > 0 {
        alert_label(crashed).chars().count() as u16
    } else {
        0
    };
    let leader_width = if matches!(context, StatusContext::Terminal) {
        LEADER_LABEL.chars().count() as u16
    } else {
        0
    };
    let reserved_width = alert_width
        .saturating_add(leader_width)
        .saturating_add(hint_width(HELP_HINT));
    let available_width = area.width.saturating_sub(reserved_width);
    spans.extend(hint_spans(
        &context.hints(available_width),
        key_color,
        label_color,
    ));
    spans.extend(hint_spans(&[HELP_HINT], key_color, label_color));
    let style = if leader_pending {
        Style::default().bg(LEADER_PENDING_BACKGROUND)
    } else {
        Style::default()
    };
    frame.render_widget(Paragraph::new(Line::from(spans)).style(style), area);
    if crashed > 0 {
        render_alert(frame, area, crashed, leader_pending);
    }
}

/// Draws the crashed-process count pinned to the right of the status row.
fn render_alert(frame: &mut Frame, area: Rect, crashed: usize, leader_pending: bool) {
    let label = alert_label(crashed);
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
    let style = if leader_pending {
        Style::default()
            .fg(LEADER_PENDING_FOREGROUND)
            .bg(LEADER_PENDING_BACKGROUND)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(ALERT_COLOR)
            .add_modifier(Modifier::BOLD)
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(label, style))),
        alert,
    );
}

/// Builds the crashed-process alert label.
fn alert_label(crashed: usize) -> String {
    format!("{ALERT_GLYPH} {crashed} crashed ")
}

/// Selects process hints that fit, retaining lifecycle actions before secondary
/// navigation and configuration actions while preserving display order.
fn process_hints(available_width: u16) -> Vec<Hint> {
    prioritized_hints(PROCESS_HINTS, PROCESS_HINT_PRIORITY, available_width)
}

/// Selects hints by retention priority, then restores their canonical order.
fn prioritized_hints(canonical: &[Hint], priority: &[Hint], available_width: u16) -> Vec<Hint> {
    let mut remaining = available_width;
    let mut selected = Vec::new();
    for hint in priority {
        let width = hint_width(*hint);
        if width <= remaining {
            selected.push(*hint);
            remaining -= width;
        }
    }
    canonical
        .iter()
        .filter(|hint| selected.contains(hint))
        .copied()
        .collect()
}

/// Rendered width of one compact hint, including its trailing separation.
fn hint_width((key, label): Hint) -> u16 {
    format!("{key} {label}{HINT_GAP}").chars().count() as u16
}

/// Builds the styled key/label spans for a set of hints.
fn hint_spans(hints: &[Hint], key_color: Color, label_color: Color) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (key, label) in hints {
        spans.push(Span::styled(
            format!("{key} "),
            Style::default().fg(key_color),
        ));
        spans.push(Span::styled(
            format!("{label}{HINT_GAP}"),
            Style::default().fg(label_color),
        ));
    }
    spans
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;

    /// A wide process row keeps navigation, lifecycle, and configuration hints.
    #[test]
    fn a_wide_process_row_keeps_every_hint() {
        assert_eq!(process_hints(u16::MAX), PROCESS_HINTS);
    }

    /// The add action remains visible when only the highest-priority process
    /// actions fit.
    #[test]
    fn a_narrow_process_row_keeps_the_add_hint() {
        let width = hint_width(START_STOP_HINT) + hint_width(ADD_HINT);

        assert_eq!(process_hints(width), vec![START_STOP_HINT, ADD_HINT]);
    }

    #[test]
    fn shows_a_crashed_alert_pinned_right() {
        let mut terminal = Terminal::new(TestBackend::new(60, 1)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), StatusContext::Process, 2, None, false))
            .unwrap();
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn no_alert_when_nothing_has_crashed() {
        let mut terminal = Terminal::new(TestBackend::new(60, 1)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), StatusContext::Process, 0, None, false))
            .unwrap();
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn a_project_row_shows_its_own_hints() {
        let mut terminal = Terminal::new(TestBackend::new(60, 1)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), StatusContext::Project, 0, None, false))
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
                    false,
                )
            })
            .unwrap();
        insta::assert_snapshot!(terminal.backend());
    }

    /// The pending leader chord fills the status row with its active-mode color.
    #[test]
    fn a_pending_leader_chord_highlights_the_status_row() {
        let width = 60;
        let mut terminal = Terminal::new(TestBackend::new(width, 1)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), StatusContext::Terminal, 1, None, true))
            .unwrap();

        let buffer = terminal.backend().buffer();
        for x in 0..width {
            let cell = buffer.cell((x, 0)).unwrap();
            assert_eq!(cell.bg, LEADER_PENDING_BACKGROUND);
        }
    }

    /// An asynchronous notice retains the pending leader chord's full-row cue.
    #[test]
    fn a_notice_keeps_the_pending_leader_cue_visible() {
        let width = 60;
        let mut terminal = Terminal::new(TestBackend::new(width, 1)).unwrap();
        terminal
            .draw(|frame| {
                render(
                    frame,
                    frame.area(),
                    StatusContext::Terminal,
                    0,
                    Some("agent: finished"),
                    true,
                )
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        assert!(buffer.content.iter().any(|cell| cell.symbol() == "!"));
        for x in 0..width {
            let cell = buffer.cell((x, 0)).unwrap();
            assert_eq!(cell.bg, LEADER_PENDING_BACKGROUND);
        }
    }
}
