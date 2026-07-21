use ratatui::style::{Color, Style};

use crate::domain::process::{ProcessKind, ProcessState};

/// Border color when a pane has focus.
pub const FOCUS_BORDER_COLOR: Color = Color::Cyan;
/// Border color when a pane does not have focus.
pub const IDLE_BORDER_COLOR: Color = Color::DarkGray;

/// Border style for a pane, keyed on whether it currently has focus.
pub fn border_style(focused: bool) -> Style {
    let color = if focused {
        FOCUS_BORDER_COLOR
    } else {
        IDLE_BORDER_COLOR
    };
    Style::default().fg(color)
}

/// Color of a sidebar section header.
pub const HEADER_COLOR: Color = Color::Gray;
/// Color of the active/total count badge next to a section header.
pub const COUNT_COLOR: Color = Color::DarkGray;
/// Color of a process's secondary description line.
pub const DESCRIPTION_COLOR: Color = Color::DarkGray;
/// Foreground of the selected sidebar item.
pub const SELECTED_COLOR: Color = Color::White;
/// Marker drawn to the left of the selected item.
pub const SELECTION_MARKER: &str = "▎";

/// Status dot glyph and color for a process lifecycle state.
pub fn status_indicator(state: ProcessState) -> (&'static str, Color) {
    match state {
        ProcessState::Running => ("●", Color::Green),
        ProcessState::Paused => ("‖", Color::Cyan),
        ProcessState::Restarting => ("◐", Color::Yellow),
        ProcessState::Crashed => ("●", Color::Red),
        ProcessState::Pending | ProcessState::Exited => ("○", Color::DarkGray),
    }
}

/// Uppercase sidebar section title for a process kind.
pub fn section_title(kind: ProcessKind) -> &'static str {
    match kind {
        ProcessKind::Agent => "AGENTS",
        ProcessKind::Terminal => "TERMINALS",
        ProcessKind::Command => "COMMANDS",
    }
}
