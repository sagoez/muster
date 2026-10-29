use std::ops::ControlFlow;

use cliclack::{ProgressBar, spinner};
use console::Term;
use strum::EnumMessage;
use unicode_width::UnicodeWidthStr;

use super::{action::Action, interrupt_guard::InterruptGuard, settle_watch::SettleWatch};
use crate::adapter::service::ProcessStatus;

/// Between the processes a spinner names.
const PROCESS_SEPARATOR: &str = ", ";
/// Ends a name cut to fit.
const ELLIPSIS: &str = "…";
/// Columns the spinner glyph, its gap and a margin take before the message.
const SPINNER_COLUMNS: usize = 4;
/// Width assumed when the terminal's cannot be read.
const FALLBACK_WIDTH: usize = 80;

/// The spinner a person watches while a command waits for its processes: one
/// line naming what is still on its way, cleared once nothing is, so the
/// outcomes print in its place. While it is drawn, Ctrl-C ends the wait
/// rather than the process, so the frame can close cleanly. An inert one
/// draws and holds nothing.
pub struct Spinners {
    /// What turns Ctrl-C into a flag, while it draws at all.
    guard: Option<InterruptGuard>,
    /// The spinner on screen, while something is still on its way.
    spinner: Option<ProgressBar>,
}

impl Spinners {
    /// Spinners drawn on the terminal when `drawn`, inert otherwise.
    pub fn new(drawn: bool) -> Self {
        Self {
            guard: drawn.then(InterruptGuard::arm),
            spinner: None,
        }
    }

    /// Takes the spinner off the screen, if one is there.
    fn clear(&mut self) {
        if let Some(spinner) = self.spinner.take() {
            spinner.clear();
        }
    }
}

impl SettleWatch for Spinners {
    fn sending(&mut self) -> ControlFlow<()> {
        match &self.guard {
            Some(guard) if guard.interrupted() => ControlFlow::Break(()),
            _ => ControlFlow::Continue(()),
        }
    }

    fn waiting(&mut self, action: Action, waiting: &[&ProcessStatus]) -> ControlFlow<()> {
        let Some(guard) = &self.guard else {
            return ControlFlow::Continue(());
        };
        if guard.interrupted() {
            self.clear();
            return ControlFlow::Break(());
        }
        if waiting.is_empty() {
            self.clear();
            return ControlFlow::Continue(());
        }
        let names: Vec<String> = waiting
            .iter()
            .map(|process| process.name().to_string())
            .collect();
        let width = Term::stderr()
            .size_checked()
            .map_or(FALLBACK_WIDTH, |(_, cols)| usize::from(cols));
        let message = spinner_message(
            action.get_message().unwrap_or_default(),
            &names,
            width.saturating_sub(SPINNER_COLUMNS),
        );
        match &self.spinner {
            Some(spinner) => spinner.set_message(message),
            None => {
                let started = spinner();
                started.start(message);
                self.spinner = Some(started);
            },
        }
        ControlFlow::Continue(())
    }
}

impl Drop for Spinners {
    /// Clears a spinner a failed command left on screen.
    fn drop(&mut self) {
        self.clear();
    }
}

/// `verb` and the `names` it is under way for, within `width` columns: as
/// many names as fit, then how many more there are, and a first name cut
/// short when even it does not fit.
pub(super) fn spinner_message(verb: &str, names: &[String], width: usize) -> String {
    let mut message = verb.to_string();
    for (shown, name) in names.iter().enumerate() {
        let separator = if shown == 0 { " " } else { PROCESS_SEPARATOR };
        let rest = names.len() - shown - 1;
        let more = if rest == 0 {
            String::new()
        } else {
            format!(" +{rest} more")
        };
        let candidate = format!("{message}{separator}{name}");
        if UnicodeWidthStr::width(format!("{candidate}{more}").as_str()) <= width {
            message = candidate;
            continue;
        }
        let remaining = names.len() - shown;
        if shown > 0 {
            return format!("{message} +{remaining} more");
        }
        return cut(&format!("{message}{separator}{name}"), width);
    }
    message
}

/// `text` cut to `width` columns, ending with an ellipsis when cut.
fn cut(text: &str, width: usize) -> String {
    if UnicodeWidthStr::width(text) <= width {
        return text.to_string();
    }
    let room = width.saturating_sub(UnicodeWidthStr::width(ELLIPSIS));
    let mut kept = String::new();
    for c in text.chars() {
        if UnicodeWidthStr::width(format!("{kept}{c}").as_str()) > room {
            break;
        }
        kept.push(c);
    }
    format!("{kept}{ELLIPSIS}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The processes `raw` names.
    fn names(raw: &[&str]) -> Vec<String> {
        raw.iter().map(ToString::to_string).collect()
    }

    /// One action word, then the names, as many as fit, counting the rest;
    /// the message never exceeds its width.
    #[test]
    fn the_message_names_what_fits_and_counts_the_rest() {
        let three = names(&["web", "worker", "broken"]);
        assert_eq!(
            spinner_message("starting", &three, 80),
            "starting web, worker, broken"
        );
        assert_eq!(
            spinner_message("starting", &three, 24),
            "starting web +2 more"
        );
        assert_eq!(
            spinner_message("stopping", &names(&["a-very-long-process-name"]), 16),
            "stopping a-very…"
        );
        for width in 10..40 {
            let message = spinner_message("starting", &three, width);
            assert!(
                UnicodeWidthStr::width(message.as_str()) <= width,
                "{width}: {message}"
            );
        }
    }
}
