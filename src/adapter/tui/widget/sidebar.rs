use ratatui::{
    Frame,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};

use super::theme;
use crate::{
    application::Workspace,
    domain::process::{Process, ProcessKind},
};

/// Accent color of the selection marker.
const MARKER_COLOR: Color = Color::Blue;
/// Glyph marking an expanded project (its processes are shown).
const EXPANDED_GLYPH: &str = "▾";
/// Glyph marking a collapsed project.
const COLLAPSED_GLYPH: &str = "▸";
/// Indent applied to a section nested under the active project header.
const SECTION_INDENT: &str = "  ";
/// Extra indent aligning a description under its process name.
const DESCRIPTION_INDENT: &str = "    ";
/// Suffix marking a process that will not auto-start with its workspace.
const MANUAL_MARKER: &str = "  manual";
/// Rule drawn between a section title and its count badge.
const SECTION_RULE: &str = "─";
/// Blank column on each side of the section rule.
const RULE_MARGIN: usize = 1;

/// The current sidebar selection: a process in the active project, or one of the
/// collapsed other-project rows.
pub enum SidebarSelection {
    /// The nth process in the active project.
    Process(usize),
    /// The nth collapsed other-project row.
    Project(usize),
}

/// Renders the sidebar as a project tree: the active project expanded into its
/// AGENTS / TERMINALS / COMMANDS sections (with counts, status dots, and the
/// current selection), and every other registered project collapsed below.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    workspace: &Workspace,
    focused: bool,
    active_project: &str,
    other_projects: &[String],
    selection: SidebarSelection,
) {
    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(theme::border_style(focused));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(
        Paragraph::new(build_lines(
            workspace,
            active_project,
            other_projects,
            selection,
            inner.width as usize,
        )),
        inner,
    );
}

/// Builds the tree lines: the active project header, its sections and items,
/// then a collapsed row per other project. `selected_process` marks a process in
/// the active project; `selected_project` marks a collapsed project row.
fn build_lines(
    workspace: &Workspace,
    active_project: &str,
    other_projects: &[String],
    selection: SidebarSelection,
    width: usize,
) -> Vec<Line<'static>> {
    let (selected_process, selected_project) = match selection {
        SidebarSelection::Process(index) => (Some(index), None),
        SidebarSelection::Project(index) => (None, Some(index)),
    };
    let mut lines = Vec::new();
    lines.push(project_line(EXPANDED_GLYPH, active_project, true, false));

    let processes = workspace.processes();
    let mut current: Option<ProcessKind> = None;
    for (index, process) in processes.iter().enumerate() {
        let kind = *process.kind();
        if current != Some(kind) {
            if current.is_some() {
                lines.push(Line::default());
            }
            current = Some(kind);
            lines.push(header_line(kind, processes, SECTION_INDENT, width));
        }
        push_item_lines(
            &mut lines,
            process,
            selected_process == Some(index),
            SECTION_INDENT,
        );
    }

    for (index, name) in other_projects.iter().enumerate() {
        lines.push(Line::default());
        lines.push(project_line(
            COLLAPSED_GLYPH,
            name,
            false,
            selected_project == Some(index),
        ));
    }
    lines
}

/// A project row: an expand/collapse glyph and the project name. Bold when it is
/// the active project; marked when it is the current sidebar selection.
fn project_line(glyph: &str, name: &str, active: bool, selected: bool) -> Line<'static> {
    let marker = if selected {
        theme::SELECTION_MARKER
    } else {
        " "
    };
    let name_style = if active || selected {
        Style::default()
            .fg(theme::SELECTED_COLOR)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme::HEADER_COLOR)
    };
    Line::from(vec![
        Span::styled(
            format!("{marker}{glyph} "),
            Style::default().fg(MARKER_COLOR),
        ),
        Span::styled(name.to_string(), name_style),
    ])
}

/// A section header: the uppercase title, a rule filling the row, and an
/// active/total count badge right-aligned to `width`.
fn header_line(
    kind: ProcessKind,
    processes: &[Process],
    indent: &str,
    width: usize,
) -> Line<'static> {
    let total = processes.iter().filter(|p| *p.kind() == kind).count();
    let active = processes
        .iter()
        .filter(|p| *p.kind() == kind && p.state().is_active())
        .count();
    let title = format!("{indent}{}", theme::section_title(kind));
    let count = format!("{active}/{total}");
    let used = title.chars().count() + count.chars().count() + RULE_MARGIN * 2;
    let rule = SECTION_RULE.repeat(width.saturating_sub(used));
    Line::from(vec![
        Span::styled(
            title,
            Style::default()
                .fg(theme::HEADER_COLOR)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(rule, Style::default().fg(theme::COUNT_COLOR)),
        Span::raw(" "),
        Span::styled(count, Style::default().fg(theme::COUNT_COLOR)),
    ])
}

/// Pushes an item's line(s): the status dot and name, plus an optional
/// description line, styled for the selected state.
fn push_item_lines(
    lines: &mut Vec<Line<'static>>,
    process: &Process,
    selected: bool,
    indent: &str,
) {
    let (glyph, color) = theme::status_indicator(*process.state());
    let marker = if selected {
        theme::SELECTION_MARKER
    } else {
        " "
    };
    let name_style = if selected {
        Style::default()
            .fg(theme::SELECTED_COLOR)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };
    let mut spans = vec![
        Span::styled(
            format!("{indent}{marker} "),
            Style::default().fg(MARKER_COLOR),
        ),
        Span::styled(format!("{glyph} "), Style::default().fg(color)),
        Span::styled(process.name().as_ref().to_string(), name_style),
    ];
    if !process.autostart() {
        spans.push(Span::styled(
            MANUAL_MARKER.to_string(),
            Style::default().fg(theme::DESCRIPTION_COLOR),
        ));
    }
    lines.push(Line::from(spans));
    if let Some(description) = process.description() {
        lines.push(Line::from(Span::styled(
            format!("{indent}{DESCRIPTION_INDENT}{description}"),
            Style::default().fg(theme::DESCRIPTION_COLOR),
        )));
    }
}

#[cfg(test)]
mod tests {
    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::domain::{
        process::{ProcessState, RestartPolicy},
        value::{CommandLine, Description, PaneId, ProcessName},
    };

    fn process(
        id: u64,
        name: &str,
        kind: ProcessKind,
        state: ProcessState,
        description: Option<&str>,
    ) -> Process {
        Process::builder()
            .id(PaneId::new(id))
            .name(ProcessName::try_new(name).unwrap())
            .kind(kind)
            .command(Some(CommandLine::try_new("true").unwrap()))
            .description(description.map(|d| Description::try_new(d).unwrap()))
            .restart(RestartPolicy::Never)
            .state(state)
            .build()
    }

    fn sample_workspace() -> Workspace {
        Workspace::builder()
            .processes(vec![
                process(
                    0,
                    "Claude Code",
                    ProcessKind::Agent,
                    ProcessState::Running,
                    None,
                ),
                process(
                    1,
                    "Codex",
                    ProcessKind::Agent,
                    ProcessState::Running,
                    Some("banner display only"),
                ),
                process(
                    2,
                    "Blank terminal",
                    ProcessKind::Terminal,
                    ProcessState::Pending,
                    None,
                ),
                process(
                    3,
                    "worker",
                    ProcessKind::Command,
                    ProcessState::Crashed,
                    None,
                ),
            ])
            .selected_index(1)
            .build()
    }

    #[test]
    fn renders_the_active_project_expanded_with_others_collapsed() {
        let workspace = sample_workspace();
        let mut terminal = Terminal::new(TestBackend::new(34, 18)).unwrap();
        terminal
            .draw(|frame| {
                render(
                    frame,
                    frame.area(),
                    &workspace,
                    true,
                    "web-api",
                    &["web-ui".to_string(), "one  canary".to_string()],
                    SidebarSelection::Process(1),
                )
            })
            .unwrap();
        insta::assert_snapshot!(terminal.backend());
    }
}
