use cliclack::{Theme, ThemeState};
use console::{Style, style};
use strum::EnumMessage;

use super::row_kind::RowKind;

/// The side bar that carries a plain line, in place of a status glyph.
const BAR_GLYPH: &str = "│";
/// The corner that ends a prompt's frame.
const BAR_END_GLYPH: &str = "└";
/// Between a frame glyph and its text.
const GLYPH_GAP: &str = "  ";
/// The spinner template, uncoloured: colour means status only.
const SPINNER_TEMPLATE: &str = "{spinner}";
/// Between the spinner and its message.
const SPINNER_GAP: &str = "  ";

/// muster's palette applied to cliclack: colour means status only, so
/// green for what passed, yellow for a warning, red for a problem and dim
/// for the guide line, with the same marks the plain layout prints.
pub struct MusterTheme;

impl Theme for MusterTheme {
    fn bar_color(&self, state: &ThemeState) -> Style {
        match state {
            ThemeState::Active | ThemeState::Submit => Style::new().dim(),
            ThemeState::Cancel => Style::new().red(),
            ThemeState::Error(_) => Style::new().yellow(),
        }
    }

    /// A question waiting for an answer is marked unstyled, an answered one
    /// green.
    fn state_symbol_color(&self, state: &ThemeState) -> Style {
        match state {
            ThemeState::Active => Style::new(),
            ThemeState::Submit => Style::new().green(),
            ThemeState::Cancel | ThemeState::Error(_) => self.bar_color(state),
        }
    }

    fn info_symbol(&self) -> String {
        self.bar_color(&ThemeState::Submit)
            .apply_to(BAR_GLYPH)
            .to_string()
    }

    fn warning_symbol(&self) -> String {
        style(RowKind::Hint.get_message().unwrap_or_default())
            .yellow()
            .to_string()
    }

    fn error_symbol(&self) -> String {
        style(RowKind::Fail.get_message().unwrap_or_default())
            .red()
            .to_string()
    }

    fn active_symbol(&self) -> String {
        style(RowKind::Ok.get_message().unwrap_or_default())
            .green()
            .to_string()
    }

    /// A cancelled prompt leaves its frame open, so the command closes it
    /// once with its own words rather than under a second, generic ending.
    fn format_footer_with_message(&self, state: &ThemeState, message: &str) -> String {
        let footer = match state {
            ThemeState::Cancel | ThemeState::Submit => BAR_GLYPH.to_string(),
            ThemeState::Active => format!("{BAR_END_GLYPH}{GLYPH_GAP}{message}"),
            ThemeState::Error(error) => format!("{BAR_END_GLYPH}{GLYPH_GAP}{error}"),
        };
        format!("{}\n", self.bar_color(state).apply_to(footer))
    }

    fn format_progress_start(&self, template: &str, grouped: bool, last: bool) -> String {
        self.format_progress_with_state(
            &format!("{SPINNER_TEMPLATE}{SPINNER_GAP}{template}"),
            grouped,
            last,
            &ThemeState::Active,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The status marks are the plain layout's, so both layouts read alike,
    /// and neither the guide nor the spinner is coloured.
    #[test]
    fn the_marks_are_the_plain_layouts() {
        console::set_colors_enabled_stderr(false);
        console::set_colors_enabled(false);
        assert_eq!(MusterTheme.active_symbol(), "◇");
        assert_eq!(MusterTheme.warning_symbol(), "▲");
        assert_eq!(MusterTheme.error_symbol(), "■");
        assert_eq!(
            MusterTheme.bar_color(&ThemeState::Active),
            MusterTheme.bar_color(&ThemeState::Submit)
        );
        assert!(
            !MusterTheme
                .format_progress_start("{msg}", false, false)
                .contains(".cyan")
        );
    }

    /// A cancelled prompt's footer is a plain bar, leaving the command to
    /// close the frame once with why.
    #[test]
    fn a_cancelled_prompt_leaves_the_frame_to_the_command() {
        console::set_colors_enabled(false);
        assert_eq!(MusterTheme.format_footer(&ThemeState::Cancel), "│\n");
        assert_eq!(
            MusterTheme.format_footer(&ThemeState::Error("too short".to_string())),
            "└  too short\n"
        );
    }
}
