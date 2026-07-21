use std::path::Path;

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};

use super::overlay;
use crate::adapter::tui::form::{Field, Form};

/// Field-label color.
const LABEL_COLOR: Color = Color::DarkGray;
/// Value color of the active field.
const ACTIVE_VALUE_COLOR: Color = Color::White;
/// Value color of an inactive field.
const IDLE_VALUE_COLOR: Color = Color::Gray;
/// Keyboard-hint color.
const HINT_COLOR: Color = Color::DarkGray;
/// Color of a validation/failure line.
const ERROR_COLOR: Color = Color::Red;
/// Failure-line prefix.
const ERROR_PREFIX: &str = "! ";
/// Marker to the left of the active field's value (matches the sidebar marker).
const ACTIVE_MARKER: &str = "▎";
/// Brackets framing a choice value.
const CHOICE_OPEN: &str = "‹ ";
const CHOICE_CLOSE: &str = " ›";
/// Keyboard hints shown at the foot of the form.
const HINT: &str = " ⏎ save   tab next   ←→ edit   esc cancel";
/// Blank rows above the fields, inside the border.
const TOP_PAD: u16 = 1;
/// Blank rows between the fields and the hints.
const GAP: u16 = 1;
/// Columns available for a field's value, beyond which it scrolls horizontally.
const FIELD_WIDTH: usize = 54;
/// Marker on the highlighted autocomplete candidate.
const CANDIDATE_MARKER: &str = "▸";
/// Color of an unhighlighted candidate.
const CANDIDATE_COLOR: Color = Color::DarkGray;
/// Color of the highlighted candidate.
const CANDIDATE_SELECTED_COLOR: Color = Color::White;

/// Draws a form overlay: a titled rounded panel with one label/value pair per
/// field, the active field marked and showing a cursor, over a dimmed backdrop.
pub fn render(frame: &mut Frame, area: Rect, form: &Form, error: Option<&str>) {
    let body = field_lines(form);
    let mut footer = Vec::new();
    if let Some(error) = error {
        footer.push(Line::from(Span::styled(
            format!(" {ERROR_PREFIX}{error}"),
            Style::default().fg(ERROR_COLOR),
        )));
    }
    footer.push(Line::from(Span::styled(
        HINT,
        Style::default().fg(HINT_COLOR),
    )));
    let title = title_line(form.title());
    let footer_height = footer.len() as u16;

    let content = body
        .iter()
        .chain(&footer)
        .chain(std::iter::once(&title))
        .map(|line| line.width())
        .max()
        .unwrap_or(0) as u16;
    let width = overlay::clamp_width(content, area);
    let height =
        (TOP_PAD + body.len() as u16 + GAP + footer_height + overlay::BORDERS).min(area.height);
    let modal = overlay::centered(width, height, area);

    overlay::dim_backdrop(frame, area);
    overlay::draw_shadow(frame, modal, area);
    frame.render_widget(Clear, modal);
    let block = overlay::panel(title);
    let inner = block.inner(modal);
    frame.render_widget(block, modal);

    let regions = Layout::vertical([
        Constraint::Length(TOP_PAD),
        Constraint::Length(body.len() as u16),
        Constraint::Length(GAP),
        Constraint::Length(footer_height),
    ])
    .split(inner);
    frame.render_widget(Paragraph::new(body), regions[1]);
    frame.render_widget(Paragraph::new(footer), regions[3]);
}

/// A label line then a value line for every field (plus an active path field's
/// autocomplete dropdown), blank-separated.
fn field_lines(form: &Form) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for (index, field) in form.fields().iter().enumerate() {
        let active = index == form.active();
        lines.push(Line::from(Span::styled(
            format!("  {}", field.label()),
            Style::default().fg(LABEL_COLOR),
        )));
        lines.push(value_line(field, active));
        if active {
            lines.extend(candidate_lines(field));
        }
        lines.push(Line::default());
    }
    lines.pop();
    lines
}

/// The dropdown lines for an active path field's autocomplete candidates, shown
/// by directory name with the highlighted one marked.
fn candidate_lines(field: &Field) -> Vec<Line<'static>> {
    let highlighted = field.highlighted();
    field
        .candidates()
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            let name = Path::new(candidate)
                .file_name()
                .and_then(|component| component.to_str())
                .unwrap_or(candidate);
            let selected = index == highlighted;
            let marker = if selected { CANDIDATE_MARKER } else { " " };
            let color = if selected {
                CANDIDATE_SELECTED_COLOR
            } else {
                CANDIDATE_COLOR
            };
            Line::from(vec![
                Span::raw("   "),
                Span::styled(marker, Style::default().fg(overlay::ACCENT_COLOR)),
                Span::raw(" "),
                Span::styled(name.to_string(), Style::default().fg(color)),
            ])
        })
        .collect()
}

/// The value line for a field: a text value with a cursor, or a bracketed
/// choice, marked when active.
fn value_line(field: &Field, active: bool) -> Line<'static> {
    let mark = if active { ACTIVE_MARKER } else { " " };
    let value_color = if active {
        ACTIVE_VALUE_COLOR
    } else {
        IDLE_VALUE_COLOR
    };
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(mark, Style::default().fg(overlay::ACCENT_COLOR)),
        Span::raw(" "),
    ];
    match field.visible(FIELD_WIDTH, active) {
        None => {
            spans.push(Span::styled(
                CHOICE_OPEN,
                Style::default().fg(overlay::ACCENT_COLOR),
            ));
            spans.push(Span::styled(
                field.value(),
                Style::default().fg(value_color),
            ));
            spans.push(Span::styled(
                CHOICE_CLOSE,
                Style::default().fg(overlay::ACCENT_COLOR),
            ));
        },
        Some((visible, cursor)) if active => {
            let chars: Vec<char> = visible.chars().collect();
            let before: String = chars.iter().take(cursor).collect();
            let at: String = chars.get(cursor).map_or(" ".to_string(), char::to_string);
            let after: String = chars.iter().skip(cursor + 1).collect();
            spans.push(Span::styled(before, Style::default().fg(value_color)));
            spans.push(Span::styled(
                at,
                Style::default()
                    .fg(value_color)
                    .add_modifier(Modifier::REVERSED),
            ));
            spans.push(Span::styled(after, Style::default().fg(value_color)));
        },
        Some((visible, _)) => {
            spans.push(Span::styled(visible, Style::default().fg(value_color)));
        },
    }
    Line::from(spans)
}

/// The title line, in the accent color.
fn title_line(title: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw(" "),
        Span::styled(
            title.to_string(),
            Style::default()
                .fg(overlay::ACCENT_COLOR)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ])
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::adapter::tui::form::Field;

    #[test]
    fn renders_a_form_with_an_active_field() {
        let form = Form::new("New project", vec![
            Field::text("Name"),
            Field::path("Folder", "~/Projects"),
        ]);
        let mut terminal = Terminal::new(TestBackend::new(56, 12)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), &form, None))
            .unwrap();
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn renders_a_path_field_with_a_dropdown() {
        let mut form = Form::new("New project", vec![Field::path("Folder", "~/w/pr")]);
        form.set_active_candidates(vec![
            "~/w/prism".to_string(),
            "~/w/proto".to_string(),
            "~/w/project".to_string(),
        ]);
        let mut terminal = Terminal::new(TestBackend::new(56, 14)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), &form, None))
            .unwrap();
        insta::assert_snapshot!(terminal.backend());
    }
}
