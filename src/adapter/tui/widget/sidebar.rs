use std::collections::HashMap;

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
    domain::{
        process::{ActivityState, Process, ProcessKind, ProcessOrigin, ProcessState},
        value::PaneId,
    },
};

/// Per-pane resource usage shown under a running process. Carries the OS pid so
/// the row is actionable (for `kill`, logs, ...), plus the tree-summed load.
#[derive(Clone, Copy, Debug, PartialEq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub(crate) struct PaneUsage {
    /// Operating-system process id of the pane's child.
    pid: u32,
    /// Tree-summed CPU use, as a percentage where 100 is one core.
    cpu_percent: f32,
    /// Tree-summed resident memory, in bytes.
    memory_bytes: u64,
}

/// Separator between the pid, memory, and CPU fields of a usage line.
const USAGE_SEPARATOR: &str = " · ";
/// Bytes per binary unit step (KiB, MiB, GiB).
const BYTE_STEP: f64 = 1024.0;
/// Binary memory unit suffixes, ascending from bytes. Labeled with the IEC binary
/// prefixes because the step is 1024 (see [`BYTE_STEP`]), not the decimal 1000.
const MEMORY_UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];

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
/// Suffix marking a runtime agent session (opened at runtime, not pinned in the
/// config), so it is distinct from a configured agent that responds to `t`.
const SESSION_MARKER: &str = "  session";
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
    /// Latest resource usage per running pane, keyed by pane id.
    usage: &'a HashMap<PaneId, PaneUsage>,
}

/// The sidebar frame: just a right border separating it from the pane.
fn sidebar_block(focused: bool) -> Block<'static> {
    Block::default()
        .borders(Borders::RIGHT)
        .border_type(theme::border_type(focused))
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
    let (lines, _, targets) = build_lines(
        state.workspace,
        state.activity_frame,
        state.active_project,
        state.other_projects,
        state.selection,
        state.usage,
        inner.width as usize,
    );
    // Match the render's scroll so a click lands on the row actually drawn there.
    let offset = scroll_offset(
        &targets,
        state.selection,
        lines.len(),
        inner.height as usize,
    );
    let row = usize::from(position.y - inner.y) + offset;
    targets.get(row).copied().flatten()
}

/// Renders the sidebar as a project tree: the active project expanded into its
/// AGENTS / TERMINALS / COMMANDS sections (with counts, status dots, and the
/// current selection), and every other registered project collapsed below.
pub fn render(frame: &mut Frame, area: Rect, state: &SidebarState<'_>) {
    let block = sidebar_block(state.focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let (lines, activity, targets) = build_lines(
        state.workspace,
        state.activity_frame,
        state.active_project,
        state.other_projects,
        state.selection,
        state.usage,
        inner.width as usize,
    );
    // Usage rows can push the tree past a short viewport, so scroll to keep the
    // selected row visible rather than let it fall off-screen unseen.
    let offset = scroll_offset(
        &targets,
        state.selection,
        lines.len(),
        inner.height as usize,
    );
    frame.render_widget(Paragraph::new(lines).scroll((offset as u16, 0)), inner);
    if inner.width == 0 {
        return;
    }
    let x = inner.x + inner.width - 1;
    for (row, glyph, color) in activity {
        let Some(row) = row.checked_sub(offset) else {
            continue;
        };
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

/// Vertical scroll that keeps the selected row visible: none while the whole
/// tree fits, otherwise the least scroll that brings the selection into view
/// without scrolling past the end.
fn scroll_offset(
    targets: &[Option<SidebarSelection>],
    selection: SidebarSelection,
    total: usize,
    viewport: usize,
) -> usize {
    if viewport == 0 || total <= viewport {
        return 0;
    }
    let selected = targets
        .iter()
        .position(|target| *target == Some(selection))
        .unwrap_or(0);
    selected.saturating_sub(viewport - 1).min(total - viewport)
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
    usage: &HashMap<PaneId, PaneUsage>,
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
            usage.get(process.id()),
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

/// Pushes an item's line(s): the status dot and name, an optional description
/// line, and a usage line while the process is running. Styled for the selected
/// state; every pushed row targets the process at `index` so clicks land on it.
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
    usage: Option<&PaneUsage>,
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
    if *process.kind() == ProcessKind::Agent && *process.origin() == ProcessOrigin::Session {
        spans.push(Span::styled(
            SESSION_MARKER.to_string(),
            Style::default().fg(theme::DESCRIPTION_COLOR),
        ));
    } else if !process.autostart() {
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
    if let Some(usage) = usage {
        lines.push(Line::from(Span::styled(
            format!("{indent}{DESCRIPTION_INDENT}{}", usage_label(usage)),
            Style::default().fg(theme::DESCRIPTION_COLOR),
        )));
        targets.push(Some(SidebarSelection::Process(index)));
    }
}

/// Formats a usage line: pid, memory, and CPU percent, separated by dots.
fn usage_label(usage: &PaneUsage) -> String {
    format!(
        "{}{USAGE_SEPARATOR}{}{USAGE_SEPARATOR}{:.0}%",
        usage.pid(),
        format_memory(*usage.memory_bytes()),
        usage.cpu_percent(),
    )
}

/// Renders `bytes` as a compact binary-unit string (for example `42 MiB`).
fn format_memory(bytes: u64) -> String {
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= BYTE_STEP && unit < MEMORY_UNITS.len() - 1 {
        value /= BYTE_STEP;
        unit += 1;
    }
    let suffix = MEMORY_UNITS[unit];
    if unit == 0 {
        format!("{bytes} {suffix}")
    } else {
        format!("{value:.0} {suffix}")
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
        process::{ProcessOrigin, ProcessState, RestartPolicy},
        value::{CommandLine, Description, PaneId, ProcessName},
    };

    /// A shared empty usage map for tests that do not exercise usage rendering.
    fn no_usage() -> &'static HashMap<PaneId, PaneUsage> {
        static EMPTY: std::sync::OnceLock<HashMap<PaneId, PaneUsage>> = std::sync::OnceLock::new();
        EMPTY.get_or_init(HashMap::new)
    }

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
            .usage(no_usage())
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
                    .usage(no_usage())
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
                    .usage(no_usage())
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
                    .usage(no_usage())
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
                    .usage(no_usage())
                    .build();
                render(frame, frame.area(), &state)
            })
            .unwrap();

        let marker = terminal.backend().buffer().cell((14, 2)).unwrap();
        assert_eq!(marker.symbol(), ActivityFrame::initial().glyph());
        assert_eq!(marker.fg, WORKING_COLOR);
    }

    /// A runtime agent session is tagged, so it reads as distinct from a
    /// configured agent that answers to `t`.
    #[test]
    fn a_session_agent_is_tagged_as_a_session() {
        let session = Process::builder()
            .id(PaneId::new(0))
            .name(ProcessName::try_new("claude").unwrap())
            .kind(ProcessKind::Agent)
            .command(Some(CommandLine::try_new("claude").unwrap()))
            .description(None)
            .restart(RestartPolicy::Never)
            .state(ProcessState::Running)
            .origin(ProcessOrigin::Session)
            .build();
        let workspace = Workspace::builder()
            .processes(vec![session])
            .selected_index(0)
            .build();
        let mut terminal = Terminal::new(TestBackend::new(32, 6)).unwrap();
        terminal
            .draw(|frame| {
                let state = SidebarState::builder()
                    .workspace(&workspace)
                    .activity_frame(ActivityFrame::initial())
                    .focused(true)
                    .active_project("project")
                    .other_projects(&[])
                    .selection(SidebarSelection::Process(0))
                    .usage(no_usage())
                    .build();
                render(frame, frame.area(), &state)
            })
            .unwrap();

        let rendered: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(rendered.contains("session"));
    }

    /// A paused agent retains its activity state without rendering a working
    /// marker while the child cannot make progress.
    #[test]
    fn paused_agent_activity_has_no_working_marker() {
        let mut working = process(0, "agent", ProcessKind::Agent, ProcessState::Paused, None);
        working.set_activity(ActivityState::Working);

        assert_eq!(activity_indicator(&working, ActivityFrame::initial()), None);
    }

    /// A running process with a usage sample shows its pid, memory, and CPU.
    #[test]
    fn a_running_process_shows_its_usage() {
        let worker = process(
            0,
            "worker",
            ProcessKind::Command,
            ProcessState::Running,
            None,
        );
        let workspace = Workspace::builder()
            .processes(vec![worker])
            .selected_index(0)
            .build();
        let mut usage = HashMap::new();
        usage.insert(
            PaneId::new(0),
            PaneUsage::builder()
                .pid(4242)
                .cpu_percent(12.0)
                .memory_bytes(44 * 1024 * 1024)
                .build(),
        );
        let mut terminal = Terminal::new(TestBackend::new(32, 8)).unwrap();
        terminal
            .draw(|frame| {
                let state = SidebarState::builder()
                    .workspace(&workspace)
                    .activity_frame(ActivityFrame::initial())
                    .focused(true)
                    .active_project("project")
                    .other_projects(&[])
                    .selection(SidebarSelection::Process(0))
                    .usage(&usage)
                    .build();
                render(frame, frame.area(), &state)
            })
            .unwrap();

        let rendered: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(rendered.contains("4242"));
        assert!(rendered.contains("44 MiB"));
        assert!(rendered.contains("12%"));
    }

    /// Memory is rendered in the largest binary unit that keeps it compact.
    #[test]
    fn format_memory_uses_binary_units() {
        assert_eq!(format_memory(512), "512 B");
        assert_eq!(format_memory(2048), "2 KiB");
        assert_eq!(format_memory(44 * 1024 * 1024), "44 MiB");
    }

    /// Hit-testing applies the same scroll offset as rendering, so clicking a
    /// visible row in a scrolled tree selects the process actually drawn there.
    #[test]
    fn hit_testing_accounts_for_the_scroll_offset() {
        let processes: Vec<Process> = (0..8)
            .map(|id| {
                process(
                    id,
                    &format!("proc{id}"),
                    ProcessKind::Command,
                    ProcessState::Running,
                    None,
                )
            })
            .collect();
        let mut usage = HashMap::new();
        for id in 0..8 {
            usage.insert(
                PaneId::new(id),
                PaneUsage::builder()
                    .pid(id as u32)
                    .cpu_percent(0.0)
                    .memory_bytes(1024)
                    .build(),
            );
        }
        let workspace = Workspace::builder()
            .processes(processes)
            .selected_index(7)
            .build();
        let others: Vec<String> = Vec::new();
        let state = SidebarState::builder()
            .workspace(&workspace)
            .activity_frame(ActivityFrame::initial())
            .focused(true)
            .active_project("project")
            .other_projects(&others)
            .selection(SidebarSelection::Process(7))
            .usage(&usage)
            .build();
        let area = Rect::new(0, 0, 32, 8);

        // The last visible row of the scrolled viewport is the selected process.
        assert_eq!(
            selection_at(&state, area, Position::new(1, 7)),
            Some(SidebarSelection::Process(7))
        );
    }

    /// When usage rows push the tree past a short viewport, the sidebar scrolls
    /// so the selected process stays visible instead of falling off-screen.
    #[test]
    fn the_selection_stays_visible_when_the_tree_overflows() {
        let processes: Vec<Process> = (0..8)
            .map(|id| {
                process(
                    id,
                    &format!("proc{id}"),
                    ProcessKind::Command,
                    ProcessState::Running,
                    None,
                )
            })
            .collect();
        let last = processes.len() - 1;
        let mut usage = HashMap::new();
        for id in 0..8 {
            usage.insert(
                PaneId::new(id),
                PaneUsage::builder()
                    .pid(id as u32)
                    .cpu_percent(0.0)
                    .memory_bytes(1024)
                    .build(),
            );
        }
        let workspace = Workspace::builder()
            .processes(processes)
            .selected_index(last)
            .build();
        let mut terminal = Terminal::new(TestBackend::new(32, 8)).unwrap();
        terminal
            .draw(|frame| {
                let state = SidebarState::builder()
                    .workspace(&workspace)
                    .activity_frame(ActivityFrame::initial())
                    .focused(true)
                    .active_project("project")
                    .other_projects(&[])
                    .selection(SidebarSelection::Process(last))
                    .usage(&usage)
                    .build();
                render(frame, frame.area(), &state)
            })
            .unwrap();

        let rendered: String = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect();
        assert!(
            rendered.contains("proc7"),
            "the selected process should be scrolled into view"
        );
    }
}
