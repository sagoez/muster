use std::path::Path;

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};

use super::overlay;
use crate::domain::{agent_session::AgentSession, process::AgentTool};

/// Title shown when launching a disposable session (the `A` picker).
const LAUNCH_TITLE: &str = "Launch agent";
/// Title shown when adding a persisted configured agent (the `a` agent flow).
const CONFIGURE_TITLE: &str = "Add agent";
/// Heading above recently closed resumable sessions.
const RECENT_HEADING: &str = "Recent sessions";
/// Heading above the provider presets when launching a disposable session.
const LAUNCH_NEW_HEADING: &str = "New session";
/// Heading above the provider presets when adding a configured agent.
const CONFIGURE_NEW_HEADING: &str = "Provider";
/// Marker to the left of the highlighted row.
const ACTIVE_MARKER: &str = "▎";
/// Empty marker occupying the same width as [`ACTIVE_MARKER`].
const IDLE_MARKER: &str = " ";
/// Separator between a recent name and provider label.
const TOOL_SEPARATOR: &str = "  ";
/// Footer actions when launching a disposable session.
const LAUNCH_HINT: &str = " ⏎ launch   e customize   esc cancel";
/// Footer actions when adding a configured agent.
const CONFIGURE_HINT: &str = " ⏎ select   esc cancel";
/// Title shown when picking a conversation to pin as a configured agent.
const PIN_TITLE: &str = "Pin a conversation";
/// Footer actions when picking a conversation to pin.
const PIN_HINT: &str = " ⏎ pin   esc cancel";
/// Heading above the list of resumable conversations to pin.
const CONVERSATIONS_HEADING: &str = "Conversations";
/// The configure-menu row that opens the conversation list.
const FROM_CONVERSATION_LABEL: &str = "Resume a conversation…";
/// Heading color.
const HEADING_COLOR: Color = Color::DarkGray;
/// Highlighted-row color.
const ACTIVE_COLOR: Color = Color::White;
/// Unselected-row color.
const IDLE_COLOR: Color = Color::Gray;
/// Secondary provider label color.
const TOOL_COLOR: Color = Color::DarkGray;
/// Blank rows above content and before the footer.
const GAP: u16 = 1;

/// One selectable row in the agent launcher.
#[derive(Clone)]
pub enum AgentPickerItem {
    /// A durable provider conversation that can be resumed.
    Recent(Box<AgentSession>),
    /// A fresh provider session.
    New(AgentTool),
    /// The configure-menu row that drills into the conversation list.
    FromConversation,
    /// A resumable conversation offered for pinning, labeled with its folder.
    Conversation(Box<AgentSession>),
}

/// What the picker is for, which selects its title, provider heading, and hints.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AgentPickerPurpose {
    /// Launch a disposable session (the `A` picker): recent sessions above presets.
    Launch,
    /// Add a persisted configured agent (the `a` agent flow): provider presets only.
    Configure,
    /// Pick an existing conversation to pin as a configured agent in this workspace.
    PinConversation,
}

impl AgentPickerPurpose {
    /// Border title for this purpose.
    fn title(self) -> &'static str {
        match self {
            Self::Launch => LAUNCH_TITLE,
            Self::Configure => CONFIGURE_TITLE,
            Self::PinConversation => PIN_TITLE,
        }
    }

    /// Footer hint for this purpose.
    fn hint(self) -> &'static str {
        match self {
            Self::Launch => LAUNCH_HINT,
            Self::Configure => CONFIGURE_HINT,
            Self::PinConversation => PIN_HINT,
        }
    }

    /// Heading above this purpose's non-recent rows.
    fn new_heading(self) -> &'static str {
        match self {
            Self::Launch => LAUNCH_NEW_HEADING,
            Self::Configure => CONFIGURE_NEW_HEADING,
            Self::PinConversation => CONVERSATIONS_HEADING,
        }
    }
}

/// Draws a quick picker with recent sessions above fresh provider presets.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    items: &[AgentPickerItem],
    selected: usize,
    error: Option<&str>,
    purpose: AgentPickerPurpose,
) {
    let rows = picker_lines(items, selected, purpose);
    let mut footer = Vec::new();
    if let Some(error) = error {
        footer.push(Line::from(Span::styled(
            format!(" ! {error}"),
            Style::default().fg(Color::Red),
        )));
    }
    footer.push(Line::from(Span::styled(
        purpose.hint(),
        Style::default().fg(TOOL_COLOR),
    )));
    let title = Line::from(vec![
        Span::raw(" "),
        Span::styled(
            purpose.title(),
            Style::default()
                .fg(overlay::ACCENT_COLOR)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ]);
    let content = rows
        .iter()
        .chain(&footer)
        .chain(std::iter::once(&title))
        .map(|line| line.width())
        .max()
        .unwrap_or(0) as u16;
    let width = overlay::clamp_width(content, area);
    let footer_height = footer.len() as u16;
    let chrome = GAP + GAP + footer_height + overlay::BORDERS;
    let height = chrome.saturating_add(rows.len() as u16).min(area.height);
    let modal = overlay::centered(width, height, area);

    overlay::dim_backdrop(frame, area);
    overlay::draw_shadow(frame, modal, area);
    frame.render_widget(Clear, modal);
    let block = overlay::panel(title);
    let inner = block.inner(modal);
    frame.render_widget(block, modal);
    let list_height = inner.height.saturating_sub(GAP + GAP + footer_height);
    let regions = Layout::vertical([
        Constraint::Length(GAP),
        Constraint::Length(list_height),
        Constraint::Length(GAP),
        Constraint::Length(footer_height),
    ])
    .split(inner);
    let selected_line = selected_line(items, selected);
    let offset = scroll_offset(selected_line, rows.len(), list_height as usize) as u16;
    frame.render_widget(Paragraph::new(rows).scroll((offset, 0)), regions[1]);
    frame.render_widget(Paragraph::new(footer), regions[3]);
}

/// Returns the rendered line containing the selected item, accounting for
/// section headings and the separator between recent and new sessions.
fn selected_line(items: &[AgentPickerItem], selected: usize) -> usize {
    let recent = items
        .iter()
        .take_while(|item| matches!(item, AgentPickerItem::Recent(_)))
        .count();
    let selected = selected.min(items.len().saturating_sub(1));
    if recent > 0 && selected >= recent {
        selected + 3
    } else {
        selected + 1
    }
}

/// Returns the vertical offset that keeps `selected` inside a viewport with
/// `visible` rows.
fn scroll_offset(selected: usize, len: usize, visible: usize) -> usize {
    if visible == 0 || len <= visible {
        return 0;
    }
    selected.saturating_sub(visible - 1).min(len - visible)
}

/// Builds section headings and selectable rows in display order.
fn picker_lines(
    items: &[AgentPickerItem],
    selected: usize,
    purpose: AgentPickerPurpose,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let recent = items
        .iter()
        .take_while(|item| matches!(item, AgentPickerItem::Recent(_)))
        .count();
    if recent > 0 {
        lines.push(heading(RECENT_HEADING));
        for (index, item) in items.iter().take(recent).enumerate() {
            lines.push(item_line(item, index == selected));
        }
        lines.push(Line::default());
    }
    lines.push(heading(purpose.new_heading()));
    for (index, item) in items.iter().enumerate().skip(recent) {
        lines.push(item_line(item, index == selected));
    }
    lines
}

/// Builds one subdued section heading.
fn heading(label: &str) -> Line<'static> {
    Line::from(Span::styled(
        format!("  {label}"),
        Style::default()
            .fg(HEADING_COLOR)
            .add_modifier(Modifier::BOLD),
    ))
}

/// Builds one recent or provider row with a stable selection marker.
fn item_line(item: &AgentPickerItem, selected: bool) -> Line<'static> {
    let marker = if selected { ACTIVE_MARKER } else { IDLE_MARKER };
    let style = Style::default()
        .fg(if selected { ACTIVE_COLOR } else { IDLE_COLOR })
        .add_modifier(if selected {
            Modifier::BOLD
        } else {
            Modifier::empty()
        });
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(marker, Style::default().fg(overlay::ACCENT_COLOR)),
        Span::raw(" "),
    ];
    match item {
        AgentPickerItem::Recent(session) => {
            spans.push(Span::styled(session.name().as_ref().to_string(), style));
            spans.push(Span::raw(TOOL_SEPARATOR));
            spans.push(Span::styled(
                session.tool().to_string(),
                Style::default().fg(TOOL_COLOR),
            ));
        },
        AgentPickerItem::Conversation(session) => {
            spans.push(Span::styled(session.name().as_ref().to_string(), style));
            spans.push(Span::raw(TOOL_SEPARATOR));
            spans.push(Span::styled(
                conversation_folder_label(session),
                Style::default().fg(TOOL_COLOR),
            ));
            spans.push(Span::raw(TOOL_SEPARATOR));
            spans.push(Span::styled(
                session.tool().to_string(),
                Style::default().fg(TOOL_COLOR),
            ));
        },
        AgentPickerItem::New(tool) => spans.push(Span::styled(tool.to_string(), style)),
        AgentPickerItem::FromConversation => {
            spans.push(Span::styled(FROM_CONVERSATION_LABEL, style));
        },
    }
    Line::from(spans)
}

/// A concise label for where a conversation actually ran, and where pinning will
/// launch it: the name of its effective working directory (its working dir
/// resolved against its workspace, else the workspace folder), so conversations
/// from one workspace but different directories are told apart. Falls back to the
/// config path when there is no such folder.
fn conversation_folder_label(session: &AgentSession) -> String {
    session
        .effective_working_dir()
        .as_deref()
        .and_then(Path::file_name)
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| session.project().to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ratatui::{Terminal, backend::TestBackend};

    use super::*;
    use crate::domain::{
        agent_session::{AgentSessionId, AgentSessionState},
        value::{CommandLine, ProcessName},
    };

    /// Builds one closed session row for picker rendering tests.
    fn recent_session(name: &str) -> AgentSession {
        AgentSession::builder()
            .id(AgentSessionId::generate().unwrap())
            .name(ProcessName::try_new(name).unwrap())
            .tool(AgentTool::Codex)
            .project(PathBuf::from("/repo/muster.yml"))
            .launch_command(CommandLine::try_new("codex").unwrap())
            .state(AgentSessionState::Closed)
            .build()
    }

    /// Recent history and fresh providers are visually separated.
    #[test]
    fn renders_recent_and_new_sections() {
        let items = vec![
            AgentPickerItem::Recent(Box::new(recent_session("Ada"))),
            AgentPickerItem::New(AgentTool::Claude),
        ];
        let mut terminal = Terminal::new(TestBackend::new(56, 14)).unwrap();
        terminal
            .draw(|frame| {
                render(
                    frame,
                    frame.area(),
                    &items,
                    0,
                    None,
                    AgentPickerPurpose::Launch,
                );
            })
            .unwrap();
        let screen = terminal.backend().to_string();
        assert!(screen.contains(RECENT_HEADING));
        assert!(screen.contains(LAUNCH_NEW_HEADING));
        assert!(screen.contains("Ada"));
    }

    /// The configure purpose lists only providers under its own title and heading,
    /// with no recent-session section.
    #[test]
    fn configure_purpose_shows_providers_without_recents() {
        let items = AgentTool::options()
            .map(AgentPickerItem::New)
            .collect::<Vec<_>>();
        let mut terminal = Terminal::new(TestBackend::new(56, 16)).unwrap();
        terminal
            .draw(|frame| {
                render(
                    frame,
                    frame.area(),
                    &items,
                    0,
                    None,
                    AgentPickerPurpose::Configure,
                );
            })
            .unwrap();
        let screen = terminal.backend().to_string();
        assert!(screen.contains(CONFIGURE_TITLE));
        assert!(screen.contains(CONFIGURE_NEW_HEADING));
        assert!(!screen.contains(RECENT_HEADING));
    }

    /// A clipped picker scrolls to its selected provider while retaining the
    /// pinned launch footer.
    #[test]
    fn a_short_picker_keeps_the_selected_provider_visible() {
        let mut items = ["Ada", "Grace", "Margaret"]
            .into_iter()
            .map(recent_session)
            .map(Box::new)
            .map(AgentPickerItem::Recent)
            .collect::<Vec<_>>();
        items.extend(AgentTool::options().map(AgentPickerItem::New));
        let selected = items.len() - 1;
        let mut terminal = Terminal::new(TestBackend::new(56, 8)).unwrap();

        terminal
            .draw(|frame| {
                render(
                    frame,
                    frame.area(),
                    &items,
                    selected,
                    None,
                    AgentPickerPurpose::Launch,
                );
            })
            .unwrap();

        let screen = terminal.backend().to_string();
        assert!(screen.contains(&AgentTool::Custom.to_string()));
        assert!(screen.contains("launch"));
    }
}
