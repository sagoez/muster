use getset::Getters;
use ratatui::{
    Frame,
    layout::{Position, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
};
use typed_builder::TypedBuilder;

use super::theme;
use crate::{
    adapter::tui::activity_frame::ActivityFrame,
    application::Workspace,
    domain::process::{ActivityState, Process, ProcessKind, ProcessState},
};

/// Accent color of the selection marker.
const MARKER_COLOR: Color = Color::Blue;
/// Marker for a running process that explicitly requested user attention.
const ATTENTION_MARKER: &str = "!";
/// Color of the attention marker.
const ATTENTION_COLOR: Color = Color::Yellow;
/// Color of the animated working-agent marker.
const WORKING_COLOR: Color = Color::Cyan;
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarSelection {
    /// The nth process in the active project.
    Process(usize),
    /// The nth collapsed other-project row.
    Project(usize),
}

/// Immutable inputs needed to draw the project tree and its activity markers.
#[derive(Getters, TypedBuilder)]
#[getset(get = "pub")]
pub(crate) struct SidebarState<'a> {
    /// Processes and selection belonging to the active workspace.
    workspace: &'a Workspace,
    /// Current glyph in the working-agent animation.
    activity_frame: ActivityFrame,
    /// Whether keyboard navigation currently targets the sidebar.
    focused: bool,
    /// Display label of the expanded project.
    active_project: &'a str,
    /// Display labels of collapsed registered projects.
    other_projects: &'a [String],
    /// Selected process or collapsed project row.
    selection: SidebarSelection,
}

/// The sidebar frame: just a right border separating it from the pane.
fn sidebar_block(focused: bool) -> Block<'static> {
    Block::default()
        .borders(Borders::RIGHT)
        .border_style(theme::border_style(focused))
}

/// The sidebar row a click on `position` lands on: the process or collapsed
/// project drawn there, or `None` for headers, rules, and blank rows.
pub(crate) fn selection_at(
    state: &SidebarState<'_>,
    area: Rect,
    position: Position,
) -> Option<SidebarSelection> {
    let inner = sidebar_block(state.focused).inner(area);
    if !inner.contains(position) {
        return None;
    }
    let (_, _, targets) = build_lines(
        state.workspace,
        state.activity_frame,
        state.active_project,
        state.other_projects,
        state.selection,
        inner.width as usize,
    );
    targets
        .get(usize::from(position.y - inner.y))
        .copied()
        .flatten()
}

/// Renders the sidebar as a project tree: the active project expanded into its
/// AGENTS / TERMINALS / COMMANDS sections (with counts, status dots, and the
/// current selection), and every other registered project collapsed below.
pub fn render(frame: &mut Frame, area: Rect, state: &SidebarState<'_>) {
    let block = sidebar_block(state.focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let (lines, activity, _) = build_lines(
        state.workspace,
        state.activity_frame,
        state.active_project,
        state.other_projects,
        state.selection,
        inner.width as usize,
    );
    frame.render_widget(Paragraph::new(lines), inner);
    if inner.width == 0 {
        return;
    }
    let x = inner.x + inner.width - 1;
    for (row, glyph, color) in activity {
        let Ok(row) = u16::try_from(row) else {
            continue;
        };
        let y = inner.y.saturating_add(row);
        if y < inner.bottom() {
            frame
                .buffer_mut()
                .set_string(x, y, glyph, Style::default().fg(color));
        }
    }
}

/// Builds the tree lines: the active project header, its sections and items,
/// then a collapsed row per other project. `selected_process` marks a process
/// in the active project; `selected_project` marks a collapsed project row.
/// The returned targets run parallel to the lines and carry what a click on
/// each row selects.
#[allow(clippy::type_complexity)]
fn build_lines(
    workspace: &Workspace,
    activity_frame: ActivityFrame,
    active_project: &str,
    other_projects: &[String],
    selection: SidebarSelection,
    width: usize,
) -> (
    Vec<Line<'static>>,
    Vec<(usize, &'static str, Color)>,
    Vec<Option<SidebarSelection>>,
) {
    let (selected_process, selected_project) = match selection {
        SidebarSelection::Process(index) => (Some(index), None),
        SidebarSelection::Project(index) => (None, Some(index)),
    };
    let mut lines = Vec::new();
    let mut activity = Vec::new();
    let mut targets = Vec::new();
    lines.push(project_line(EXPANDED_GLYPH, active_project, true, false));
    targets.push(None);

    let processes = workspace.processes();
    let mut current: Option<ProcessKind> = None;
    for (index, process) in processes.iter().enumerate() {
        let kind = *process.kind();
        if current != Some(kind) {
            if current.is_some() {
                lines.push(Line::default());
                targets.push(None);
            }
            current = Some(kind);
            lines.push(header_line(kind, processes, SECTION_INDENT, width));
            targets.push(None);
        }
        push_item_lines(
            &mut lines,
            &mut targets,
            process,
            index,
            selected_process == Some(index),
            SECTION_INDENT,
            activity_frame,
            &mut activity,
        );
    }

    for (index, name) in other_projects.iter().enumerate() {
        lines.push(Line::default());
        targets.push(None);
        lines.push(project_line(
            COLLAPSED_GLYPH,
            name,
            false,
            selected_project == Some(index),
        ));
        targets.push(Some(SidebarSelection::Project(index)));
    }
    (lines, activity, targets)
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
/// description line, styled for the selected state. Every pushed row targets
/// the process at `index` so clicks land on it.
#[allow(clippy::too_many_arguments)]
fn push_item_lines(
    lines: &mut Vec<Line<'static>>,
    targets: &mut Vec<Option<SidebarSelection>>,
    process: &Process,
    index: usize,
    selected: bool,
    indent: &str,
    activity_frame: ActivityFrame,
    activity: &mut Vec<(usize, &'static str, Color)>,
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
    ];
    spans.push(Span::styled(
        process.name().as_ref().to_string(),
        name_style,
    ));
    if !process.autostart() {
        spans.push(Span::styled(
            MANUAL_MARKER.to_string(),
            Style::default().fg(theme::DESCRIPTION_COLOR),
        ));
    }
    let row = lines.len();
    lines.push(Line::from(spans));
    targets.push(Some(SidebarSelection::Process(index)));
    if let Some((glyph, color)) = activity_indicator(process, activity_frame) {
        activity.push((row, glyph, color));
    }
    if let Some(description) = process.description() {
        lines.push(Line::from(Span::styled(
            format!("{indent}{DESCRIPTION_INDENT}{description}"),
            Style::default().fg(theme::DESCRIPTION_COLOR),
        )));
        targets.push(Some(SidebarSelection::Process(index)));
    }
}

/// Returns the visible activity marker for a live process, keeping idle
/// distinct from both current work and a request for user attention.
fn activity_indicator(
    process: &Process,
    activity_frame: ActivityFrame,
) -> Option<(&'static str, Color)> {
    if !process.state().is_active() {
        return None;
    }
    match process.activity() {
        ActivityState::Idle => None,
        ActivityState::Working
            if *process.kind() == ProcessKind::Agent
                && *process.state() == ProcessState::Running =>
        {
            Some((activity_frame.glyph(), WORKING_COLOR))
        },
        ActivityState::Working => None,
        ActivityState::AwaitingInput => Some((ATTENTION_MARKER, ATTENTION_COLOR)),
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

    /// Clicks resolve to the process or collapsed project drawn on each row.
    #[test]
    fn selection_at_maps_rows_to_items() {
        let workspace = sample_workspace();
        let others = vec!["beta".to_string()];
        let state = SidebarState::builder()
            .workspace(&workspace)
            .activity_frame(ActivityFrame::initial())
            .focused(true)
            .active_project("alpha")
            .other_projects(&others)
            .selection(SidebarSelection::Process(1))
            .build();
        let area = Rect::new(0, 0, 32, 20);

        assert_eq!(selection_at(&state, area, Position::new(1, 0)), None);
        assert_eq!(
            selection_at(&state, area, Position::new(1, 2)),
            Some(SidebarSelection::Process(0))
        );
        // A description row still targets its process.
        assert_eq!(
            selection_at(&state, area, Position::new(1, 4)),
            Some(SidebarSelection::Process(1))
        );
        assert_eq!(
            selection_at(&state, area, Position::new(1, 7)),
            Some(SidebarSelection::Process(2))
        );
        assert_eq!(
            selection_at(&state, area, Position::new(1, 12)),
            Some(SidebarSelection::Project(0))
        );
        // The border column and rows past the tree are dead.
        assert_eq!(selection_at(&state, area, Position::new(31, 2)), None);
        assert_eq!(selection_at(&state, area, Position::new(1, 15)), None);
    }

    #[test]
    fn renders_the_active_project_expanded_with_others_collapsed() {
        let workspace = sample_workspace();
        let other_projects = vec!["web-ui".to_string(), "one  canary".to_string()];
        let mut terminal = Terminal::new(TestBackend::new(34, 18)).unwrap();
        terminal
            .draw(|frame| {
                let state = SidebarState::builder()
                    .workspace(&workspace)
                    .activity_frame(ActivityFrame::initial())
                    .focused(true)
                    .active_project("web-api")
                    .other_projects(&other_projects)
                    .selection(SidebarSelection::Process(1))
                    .build();
                render(frame, frame.area(), &state)
            })
            .unwrap();
        insta::assert_snapshot!(terminal.backend());
    }

    #[test]
    fn a_long_process_name_cannot_clip_the_attention_marker() {
        let mut waiting = process(
            0,
            "a process name wider than the sidebar",
            ProcessKind::Command,
            ProcessState::Running,
            None,
        );
        waiting.set_activity(ActivityState::AwaitingInput);
        let workspace = Workspace::builder()
            .processes(vec![waiting])
            .selected_index(0)
            .build();
        let mut terminal = Terminal::new(TestBackend::new(16, 5)).unwrap();
        terminal
            .draw(|frame| {
                let state = SidebarState::builder()
                    .workspace(&workspace)
                    .activity_frame(ActivityFrame::initial())
                    .focused(true)
                    .active_project("project")
                    .other_projects(&[])
                    .selection(SidebarSelection::Process(0))
                    .build();
                render(frame, frame.area(), &state)
            })
            .unwrap();

        let marker = terminal.backend().buffer().cell((14, 2)).unwrap();
        assert_eq!(marker.symbol(), ATTENTION_MARKER);
        assert_eq!(marker.fg, ATTENTION_COLOR);
    }

    #[test]
    fn ordinary_command_output_has_no_activity_marker() {
        let mut working = process(
            0,
            "worker",
            ProcessKind::Command,
            ProcessState::Running,
            None,
        );
        working.set_activity(ActivityState::Working);
        let workspace = Workspace::builder()
            .processes(vec![working])
            .selected_index(0)
            .build();
        let mut terminal = Terminal::new(TestBackend::new(16, 5)).unwrap();
        terminal
            .draw(|frame| {
                let state = SidebarState::builder()
                    .workspace(&workspace)
                    .activity_frame(ActivityFrame::initial())
                    .focused(true)
                    .active_project("project")
                    .other_projects(&[])
                    .selection(SidebarSelection::Process(0))
                    .build();
                render(frame, frame.area(), &state)
            })
            .unwrap();

        let marker = terminal.backend().buffer().cell((14, 2)).unwrap();
        assert_eq!(marker.symbol(), " ");
    }

    /// Working agents show an animated glyph at the right edge, separate from
    /// their persistent lifecycle dot.
    #[test]
    fn working_agent_activity_is_right_aligned() {
        let mut working = process(0, "agent", ProcessKind::Agent, ProcessState::Running, None);
        working.set_activity(ActivityState::Working);
        let workspace = Workspace::builder()
            .processes(vec![working])
            .selected_index(0)
            .build();
        let mut terminal = Terminal::new(TestBackend::new(16, 5)).unwrap();
        terminal
            .draw(|frame| {
                let state = SidebarState::builder()
                    .workspace(&workspace)
                    .activity_frame(ActivityFrame::initial())
                    .focused(true)
                    .active_project("project")
                    .other_projects(&[])
                    .selection(SidebarSelection::Process(0))
                    .build();
                render(frame, frame.area(), &state)
            })
            .unwrap();

        let marker = terminal.backend().buffer().cell((14, 2)).unwrap();
        assert_eq!(marker.symbol(), ActivityFrame::initial().glyph());
        assert_eq!(marker.fg, WORKING_COLOR);
    }

    /// A paused agent retains its activity state without rendering a working
    /// marker while the child cannot make progress.
    #[test]
    fn paused_agent_activity_has_no_working_marker() {
        let mut working = process(0, "agent", ProcessKind::Agent, ProcessState::Paused, None);
        working.set_activity(ActivityState::Working);

        assert_eq!(activity_indicator(&working, ActivityFrame::initial()), None);
    }
}
