use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};

use super::{overlay, theme};
use crate::domain::{process::ProcessKind, project::Project};

/// Title shown on the overlay's top border.
const TITLE: &str = "Switch project";
/// Count-badge color in the title.
const COUNT_COLOR: Color = Color::DarkGray;
/// Bar drawn to the left of the selected row (matches the sidebar marker).
const ACCENT_BAR: &str = "▎";
/// Name color of the selected row.
const SELECTED_NAME_COLOR: Color = Color::White;
/// Name color of an unselected row.
const NAME_COLOR: Color = Color::Gray;
/// Config-path color.
const PATH_COLOR: Color = Color::DarkGray;
/// Hotkey-number color of an unselected row.
const NUMBER_COLOR: Color = Color::DarkGray;
/// Marker on the currently loaded project.
const CURRENT_MARKER: &str = "●";
/// Color of the current-project marker (same green as a running dot).
const CURRENT_COLOR: Color = Color::Green;
/// Color of a switch-failure line.
const ERROR_COLOR: Color = Color::Red;
/// Failure-line prefix.
const ERROR_PREFIX: &str = "! ";
/// Keyboard-hint color.
const HINT_COLOR: Color = Color::DarkGray;
/// Separator between keyboard hints.
const HINT_SEPARATOR: &str = "   ";
/// Highest project index reachable by a number hotkey.
const MAX_HOTKEY: usize = 9;
/// Heading shown when no projects are registered.
const EMPTY_HEADING: &str = "No projects yet";
/// Guidance shown beneath the empty-state heading.
const EMPTY_HINT: &str = "press n to create one, or s to save the current setup";
/// Blank rows above the list, inside the border.
const TOP_PAD: u16 = 1;
/// Blank rows between the list and the footer hints.
const GAP: u16 = 1;
/// Color of a preview section label.
const PREVIEW_LABEL_COLOR: Color = Color::DarkGray;
/// Color of a previewed process name.
const PREVIEW_NAME_COLOR: Color = Color::Gray;
/// Column width reserved for a preview section label.
const PREVIEW_LABEL_WIDTH: usize = 10;

/// Draws the project-switcher overlay: a dimmed backdrop, a soft drop shadow,
/// and a centered rounded panel listing the registered projects. `current` is
/// the index of the project whose config is loaded now, and the list scrolls to
/// keep `selected` visible while the footer stays pinned.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    projects: &[Project],
    selected: usize,
    error: Option<&str>,
    current: Option<usize>,
    preview: &[(ProcessKind, String)],
) {
    let rows = project_rows(projects, selected, current);
    let preview_rows = preview_lines(preview);
    let footer = footer_lines(projects.len(), error);
    let title = title_line(projects.len());

    let content = rows
        .iter()
        .chain(&preview_rows)
        .chain(&footer)
        .chain(std::iter::once(&title))
        .map(|line| line.width())
        .max()
        .unwrap_or(0) as u16;
    let width = overlay::clamp_width(content, area);
    let footer_height = footer.len() as u16;
    let preview_height = preview_rows.len() as u16;
    let preview_gap = if preview_height > 0 { GAP } else { 0 };
    let chrome = TOP_PAD + GAP + preview_gap + preview_height + footer_height + overlay::BORDERS;
    let height = chrome.saturating_add(rows.len() as u16).min(area.height);
    let modal = overlay::centered(width, height, area);

    overlay::dim_backdrop(frame, area);
    overlay::draw_shadow(frame, modal, area);
    frame.render_widget(Clear, modal);
    let block = overlay::panel(title);
    let inner = block.inner(modal);
    frame.render_widget(block, modal);

    let list_height = inner
        .height
        .saturating_sub(TOP_PAD + GAP + preview_gap + preview_height + footer_height);
    let regions = Layout::vertical([
        Constraint::Length(TOP_PAD),
        Constraint::Length(list_height),
        Constraint::Length(GAP),
        Constraint::Length(preview_height),
        Constraint::Length(preview_gap),
        Constraint::Length(footer_height),
    ])
    .split(inner);
    let offset = scroll_offset(selected, rows.len(), list_height as usize) as u16;
    frame.render_widget(Paragraph::new(rows).scroll((offset, 0)), regions[1]);
    frame.render_widget(Paragraph::new(preview_rows), regions[3]);
    frame.render_widget(Paragraph::new(footer), regions[5]);
}

/// Preview lines for the highlighted project: one row per non-empty section
/// listing its process names, so you see what a project holds before switching.
fn preview_lines(preview: &[(ProcessKind, String)]) -> Vec<Line<'static>> {
    [
        ProcessKind::Agent,
        ProcessKind::Terminal,
        ProcessKind::Command,
    ]
    .into_iter()
    .filter_map(|kind| {
        let names: Vec<&str> = preview
            .iter()
            .filter(|(item_kind, _)| *item_kind == kind)
            .map(|(_, name)| name.as_str())
            .collect();
        if names.is_empty() {
            return None;
        }
        Some(Line::from(vec![
            Span::styled(
                format!("  {:<PREVIEW_LABEL_WIDTH$}", theme::section_title(kind)),
                Style::default().fg(PREVIEW_LABEL_COLOR),
            ),
            Span::styled(names.join(", "), Style::default().fg(PREVIEW_NAME_COLOR)),
        ]))
    })
    .collect()
}

/// The scrollable rows: one per project, or the empty-state guidance.
fn project_rows(
    projects: &[Project],
    selected: usize,
    current: Option<usize>,
) -> Vec<Line<'static>> {
    if projects.is_empty() {
        return vec![
            text_line(EMPTY_HEADING, NAME_COLOR, true),
            Line::default(),
            text_line(EMPTY_HINT, PATH_COLOR, false),
        ];
    }
    let name_width = projects
        .iter()
        .map(|project| project.name().as_ref().chars().count())
        .max()
        .unwrap_or(0);
    let number_width = projects.len().to_string().len();
    projects
        .iter()
        .enumerate()
        .map(|(index, project)| {
            project_line(
                index,
                project,
                index == selected,
                current == Some(index),
                name_width,
                number_width,
            )
        })
        .collect()
}

/// One project row: accent bar, hotkey number, name, current marker, and path.
fn project_line(
    index: usize,
    project: &Project,
    selected: bool,
    current: bool,
    name_width: usize,
    number_width: usize,
) -> Line<'static> {
    let bar = if selected { ACCENT_BAR } else { " " };
    let number_color = if selected {
        overlay::ACCENT_COLOR
    } else {
        NUMBER_COLOR
    };
    let name_style = if selected {
        Style::default()
            .fg(SELECTED_NAME_COLOR)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(NAME_COLOR)
    };
    let marker = if current { CURRENT_MARKER } else { " " };
    Line::from(vec![
        Span::raw(" "),
        Span::styled(bar, Style::default().fg(overlay::ACCENT_COLOR)),
        Span::raw(" "),
        Span::styled(
            format!("{:>number_width$}", index + 1),
            Style::default().fg(number_color),
        ),
        Span::raw("  "),
        Span::styled(
            format!("{:<name_width$}", project.name().as_ref()),
            name_style,
        ),
        Span::raw("  "),
        Span::styled(marker, Style::default().fg(CURRENT_COLOR)),
        Span::raw(" "),
        Span::styled(
            project.config().display().to_string(),
            Style::default().fg(PATH_COLOR),
        ),
        Span::raw(" "),
    ])
}

/// The pinned footer: navigation hints, project actions, and an optional error.
fn footer_lines(count: usize, error: Option<&str>) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    if count > 0 {
        lines.push(hint_line(count));
    }
    lines.push(actions_line());
    if let Some(error) = error {
        lines.push(text_line(
            &format!("{ERROR_PREFIX}{error}"),
            ERROR_COLOR,
            false,
        ));
    }
    lines
}

/// Navigation hints, tuned to how many projects are jumpable by number.
fn hint_line(count: usize) -> Line<'static> {
    let mut hint = String::from(" ");
    if count > 1 {
        let last = count.min(MAX_HOTKEY);
        hint.push_str(&format!("1-{last} jump{HINT_SEPARATOR}"));
    }
    hint.push_str(&format!("↑↓ move{HINT_SEPARATOR}⏎ switch"));
    Line::from(Span::styled(hint, Style::default().fg(HINT_COLOR)))
}

/// Project-management action hints.
fn actions_line() -> Line<'static> {
    let actions = format!(
        " n new{HINT_SEPARATOR}s save{HINT_SEPARATOR}a add{HINT_SEPARATOR}d remove{HINT_SEPARATOR}esc ✕"
    );
    Line::from(Span::styled(actions, Style::default().fg(HINT_COLOR)))
}

/// The title line: name plus a right count badge.
fn title_line(count: usize) -> Line<'static> {
    Line::from(vec![
        Span::raw(" "),
        Span::styled(
            TITLE,
            Style::default()
                .fg(overlay::ACCENT_COLOR)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(format!("{count}"), Style::default().fg(COUNT_COLOR)),
        Span::raw(" "),
    ])
}

/// A single left-padded text line in one color, optionally bold.
fn text_line(text: &str, color: Color, bold: bool) -> Line<'static> {
    let mut style = Style::default().fg(color);
    if bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    Line::from(Span::styled(format!("  {text}"), style))
}

/// Vertical scroll offset that keeps `selected` inside a `visible`-row viewport.
fn scroll_offset(selected: usize, len: usize, visible: usize) -> usize {
    if visible == 0 || len <= visible {
        return 0;
    }
    let max_offset = len - visible;
    selected.saturating_sub(visible - 1).min(max_offset)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::domain::value::ProjectName;

    fn project(name: &str, config: &str) -> Project {
        Project::builder()
            .name(ProjectName::try_new(name).unwrap())
            .config(PathBuf::from(config))
            .build()
    }

    #[test]
    fn renders_the_switcher_overlay() {
        let projects = vec![
            project("muster", "~/Projects/muster/muster.yml"),
            project("prism", "~/Work/prism/muster.yml"),
            project("pica", "~/Work/starter/pica/muster.yml"),
        ];
        let mut terminal = Terminal::new(TestBackend::new(60, 16)).unwrap();
        terminal
            .draw(|frame| {
                render(frame, frame.area(), &projects, 0, None, Some(2), &[
                    (ProcessKind::Agent, "claude".to_string()),
                    (ProcessKind::Command, "clock".to_string()),
                ])
            })
            .unwrap();
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn renders_the_empty_state() {
        let mut terminal = Terminal::new(TestBackend::new(64, 12)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), &[], 0, None, None, &[]))
            .unwrap();
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn a_long_list_scrolls_to_keep_the_selection_visible() {
        let projects: Vec<Project> = (1..=12)
            .map(|n| {
                project(
                    &format!("project-{n:02}"),
                    &format!("~/p/{n:02}/muster.yml"),
                )
            })
            .collect();
        let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), &projects, 10, None, None, &[]))
            .unwrap();
        insta::assert_snapshot!(terminal.backend());
    }
}
