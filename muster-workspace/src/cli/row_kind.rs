use anstyle::{AnsiColor, Style};
use strum::{EnumIter, EnumMessage, EnumString};

/// Style of a mark that says something worked.
const OK_STYLE: Style = AnsiColor::Green.on_default();
/// Style of a mark that warns while the command goes on.
const HINT_STYLE: Style = AnsiColor::Yellow.on_default();
/// Style of a mark that says something failed.
const FAIL_STYLE: Style = AnsiColor::Red.on_default();
/// Style of an idle mark, of detail under a mark and of other
/// de-emphasized text.
const DIM_STYLE: Style = Style::new().dimmed();

/// What a row says about its fact: its status mark, the stream it goes to
/// and which kind of line it becomes in the interactive layout. A kind's
/// message is its mark; the unmarked kinds have none. Colour means status
/// only, so only the marks and the dimmed kinds are styled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, EnumMessage, EnumString)]
#[strum(serialize_all = "kebab-case")]
pub enum RowKind {
    /// Done or healthy.
    #[strum(message = "◇")]
    Ok,
    /// A warning; the command goes on.
    #[strum(message = "▲")]
    Hint,
    /// A problem: something failed, or the command stopped.
    #[strum(message = "■")]
    Fail,
    /// Idle: stopped, pending or paused.
    #[strum(message = "○")]
    Idle,
    /// Unstyled content.
    Plain,
    /// De-emphasized content.
    Dim,
}

impl RowKind {
    /// The style of this kind's mark, or of a whole unmarked line of it.
    pub fn style(self) -> Style {
        match self {
            Self::Ok => OK_STYLE,
            Self::Hint => HINT_STYLE,
            Self::Fail => FAIL_STYLE,
            Self::Idle | Self::Dim => DIM_STYLE,
            Self::Plain => Style::new(),
        }
    }

    /// Whether a line of this kind goes to stderr: warnings and problems do,
    /// every other line goes to stdout.
    pub fn on_stderr(self) -> bool {
        matches!(self, Self::Hint | Self::Fail)
    }

    /// The mark in its colour, or `None` for an unmarked kind.
    pub fn styled_mark(self) -> Option<String> {
        let style = self.style();
        self.get_message()
            .map(|mark| format!("{style}{mark}{style:#}"))
    }
}

/// `text` dimmed, as detail under a mark is.
pub fn dimmed(text: &str) -> String {
    format!("{DIM_STYLE}{text}{DIM_STYLE:#}")
}

#[cfg(test)]
mod tests {
    use strum::{EnumMessage, IntoEnumIterator};

    use super::RowKind;

    /// Only the status kinds carry a mark.
    #[test]
    fn only_the_status_kinds_have_a_mark() {
        for kind in RowKind::iter() {
            assert_eq!(
                kind.get_message().is_some(),
                matches!(
                    kind,
                    RowKind::Ok | RowKind::Hint | RowKind::Fail | RowKind::Idle
                ),
                "{kind:?}"
            );
        }
    }

    /// Warnings and problems go to stderr; done, idle and unmarked lines go
    /// to stdout.
    #[test]
    fn only_warnings_and_problems_go_to_stderr() {
        for kind in RowKind::iter() {
            assert_eq!(
                kind.on_stderr(),
                matches!(kind, RowKind::Hint | RowKind::Fail),
                "{kind:?}"
            );
        }
    }
}
