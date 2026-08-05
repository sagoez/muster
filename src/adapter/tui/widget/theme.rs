use ratatui::{
    style::{Color, Modifier, Style},
    widgets::BorderType,
};

use crate::domain::process::{ProcessKind, ProcessState};

/// Border color when a pane has focus.
pub const FOCUS_BORDER_COLOR: Color = Color::Cyan;
/// Border color when a pane does not have focus.
pub const IDLE_BORDER_COLOR: Color = Color::DarkGray;

/// Border style for a pane, keyed on whether it currently has focus. The
/// focused border is bold and accented; the idle border is dimmed so the
/// focused region visibly pops.
pub fn border_style(focused: bool) -> Style {
    if focused {
        Style::default()
            .fg(FOCUS_BORDER_COLOR)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(IDLE_BORDER_COLOR)
            .add_modifier(Modifier::DIM)
    }
}

/// Border line weight for a pane, keyed on focus. A thick outline on the focused
/// region reads at a glance on every terminal, where a bold or dim modifier is
/// often ignored; the idle region stays a plain single line.
pub fn border_type(focused: bool) -> BorderType {
    if focused {
        BorderType::Thick
    } else {
        BorderType::Plain
    }
}

/// Style laid over pane cells inside an active drag selection. Reversing keeps
/// the selection legible over any content the child drew.
pub fn selection_style() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
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
/// Sidebar glyph shown while a process is exiting.
const STOPPING_GLYPH: &str = "◌";
/// Sidebar color shown while a process is exiting.
const STOPPING_COLOR: Color = Color::Yellow;

/// Status dot glyph and color for a process lifecycle state.
pub fn status_indicator(state: ProcessState) -> (&'static str, Color) {
    match state {
        ProcessState::Running => ("●", Color::Green),
        ProcessState::Paused => ("‖", Color::Cyan),
        ProcessState::Stopping => (STOPPING_GLYPH, STOPPING_COLOR),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A focused border is drawn bold in the accent color.
    #[test]
    fn a_focused_border_is_bold_and_accented() {
        let style = border_style(true);

        assert_eq!(style.fg, Some(FOCUS_BORDER_COLOR));
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    /// An idle border fades: dim, in the muted color.
    #[test]
    fn an_idle_border_is_dim_and_muted() {
        let style = border_style(false);

        assert_eq!(style.fg, Some(IDLE_BORDER_COLOR));
        assert!(style.add_modifier.contains(Modifier::DIM));
    }

    /// The focused region gets a heavier outline than the idle one, the cue that
    /// survives terminals which ignore bold and dim.
    #[test]
    fn a_focused_border_is_thicker_than_an_idle_one() {
        assert_eq!(border_type(true), BorderType::Thick);
        assert_eq!(border_type(false), BorderType::Plain);
    }

    /// Stopping is visibly distinct from both running and resting states.
    #[test]
    fn stopping_has_a_transition_indicator() {
        let stopping = status_indicator(ProcessState::Stopping);

        assert_eq!(stopping, (STOPPING_GLYPH, STOPPING_COLOR));
        assert_ne!(stopping, status_indicator(ProcessState::Running));
        assert_ne!(stopping, status_indicator(ProcessState::Exited));
    }
}
