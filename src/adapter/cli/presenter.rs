//! How every command presents itself to a person: framed with cliclack's
//! intro, logs, spinners, prompts and outro at an interactive terminal, or
//! as plain lines otherwise. The choice is made once per command, by
//! [`Presenter::detect`], and every cliclack call lives here.

use std::{
    error::Error,
    io::{self, Write},
    path::Path,
};

use cliclack::{confirm, intro, log, outro, outro_cancel, select, set_theme};
use console::{Term, style};
use unicode_width::UnicodeWidthStr;

use super::{
    error::CliError,
    interrupt_guard::InterruptGuard,
    presenter_theme::MusterTheme,
    report::{Report, Row, RowKind, plain, print_error},
    spinners::Spinners,
    surroundings::Surroundings,
    workspace_questions::ask_workspace,
};
use crate::{
    adapter::{config::starter_workspace, service::ProcessStatus},
    constants::APP_NAME,
    domain::{config::WorkspaceConfig, value::ProcessName},
};

/// The question that picks a process.
const CHOOSE_PROCESS: &str = "Which process?";
/// What the plain fallback says it could not ask, choosing a process.
const CHOOSE_PROCESS_ASKED: &str = "which process";
/// The argument that answers it up front.
const CHOOSE_PROCESS_ARGUMENT: &str = "its name, as `muster ps` lists it";
/// Between a process's kind and state in the choice.
const HINT_SEPARATOR: &str = " · ";
/// Between a padded label and its detail in a framed line.
const LABEL_GAP: &str = "  ";
/// How a failed frame closes when the report carries no summary.
const FAILED_OUTRO: &str = "failed";

/// How a command presents itself: framed and asking questions at an
/// interactive terminal, plain lines that never ask anywhere else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Presenter {
    /// cliclack frames, spinners and prompts on the terminal.
    Interactive,
    /// One plain line per fact on stdout, no prompts and no spinners.
    Plain,
}

impl Presenter {
    /// The presentation of a command asked for JSON when `json` is set, from
    /// what it finds around it; the interactive one installs muster's theme.
    pub fn detect(json: bool) -> Self {
        let presenter = Self::for_surroundings(Surroundings::current(json));
        if presenter.is_interactive() {
            set_theme(MusterTheme);
        }
        presenter
    }

    /// The presentation `surroundings` call for.
    pub fn for_surroundings(surroundings: Surroundings) -> Self {
        if surroundings.interactive() {
            Self::Interactive
        } else {
            Self::Plain
        }
    }

    /// Whether it frames, spins and asks.
    pub fn is_interactive(self) -> bool {
        self == Self::Interactive
    }

    /// Opens a frame titled `title`.
    ///
    /// # Errors
    /// Returns the [`io::Error`] of a failed write.
    pub fn open(self, title: &str) -> io::Result<()> {
        match self {
            Self::Interactive => intro(style(title).bold()),
            Self::Plain => Ok(()),
        }
    }

    /// Prints `report` whole: opened by its title, its rows, then closed.
    ///
    /// # Errors
    /// Returns the [`io::Error`] of a failed write.
    pub fn show(self, report: &Report) -> io::Result<()> {
        self.open(report.title())?;
        self.finish(report)
    }

    /// Prints `report`'s rows into the frame already open, then closes it.
    ///
    /// # Errors
    /// Returns the [`io::Error`] of a failed write.
    pub fn finish(self, report: &Report) -> io::Result<()> {
        self.body(report)?;
        self.close(report)
    }

    /// Prints `report`'s rows: one framed line each, or the plain lines.
    ///
    /// # Errors
    /// Returns the [`io::Error`] of a failed write.
    pub fn body(self, report: &Report) -> io::Result<()> {
        match self {
            Self::Interactive => {
                let width = label_width(report.rows());
                report
                    .rows()
                    .iter()
                    .try_for_each(|row| framed_row(row, width))
            },
            Self::Plain => print_line(&plain(report)),
        }
    }

    /// Closes the frame with `report`'s summary, as a failure when it failed.
    ///
    /// # Errors
    /// Returns the [`io::Error`] of a failed write.
    pub fn close(self, report: &Report) -> io::Result<()> {
        match (self, report.failed()) {
            (Self::Interactive, true) => {
                outro_cancel(report.summary().as_deref().unwrap_or(FAILED_OUTRO))
            },
            (Self::Interactive, false) => outro(report.summary().as_deref().unwrap_or_default()),
            (Self::Plain, _) => Ok(()),
        }
    }

    /// Closes an open frame with `error`, or prints it in the CLI's voice on
    /// stderr.
    ///
    /// # Errors
    /// Returns the [`io::Error`] of a failed write.
    pub fn fail(self, error: &dyn Error) -> io::Result<()> {
        match self {
            Self::Interactive => {
                Term::stderr().show_cursor()?;
                outro_cancel(error)
            },
            Self::Plain => {
                print_error(APP_NAME, error);
                Ok(())
            },
        }
    }

    /// Says `line` as a step inside the open frame, or as a plain line.
    ///
    /// # Errors
    /// Returns the [`io::Error`] of a failed write.
    pub fn step(self, line: &str) -> io::Result<()> {
        match self {
            Self::Interactive => log::step(line),
            Self::Plain => print_line(line),
        }
    }

    /// Closes the open frame with `line`, or prints it as a plain line.
    ///
    /// # Errors
    /// Returns the [`io::Error`] of a failed write.
    pub fn conclude(self, line: &str) -> io::Result<()> {
        match self {
            Self::Interactive => outro(line),
            Self::Plain => print_line(line),
        }
    }

    /// The spinner a command shows while it waits for its processes; it draws
    /// nothing in the plain layout.
    pub fn spinners(self) -> Spinners {
        Spinners::new(self.is_interactive())
    }

    /// Checks that a process can be chosen by asking, before anything else
    /// is done to ask it.
    ///
    /// # Errors
    /// Returns [`CliError::CannotAsk`] naming the argument to pass, in the
    /// plain layout.
    pub fn can_choose_process(self) -> Result<(), CliError> {
        match self {
            Self::Interactive => Ok(()),
            Self::Plain => Err(CliError::CannotAsk {
                question: CHOOSE_PROCESS_ASKED,
                argument: CHOOSE_PROCESS_ARGUMENT,
            }),
        }
    }

    /// Asks which of `candidates`, processes of `project`, the command is
    /// for, showing each one's kind and state.
    ///
    /// # Errors
    /// Returns [`CliError::CannotAsk`] in the plain layout,
    /// [`CliError::NothingToChoose`] when there is no candidate, or
    /// [`CliError::Cancelled`] when the person cancels.
    pub fn choose_process(
        self,
        candidates: &[&ProcessStatus],
        project: &Path,
    ) -> Result<ProcessName, CliError> {
        self.can_choose_process()?;
        if candidates.is_empty() {
            return Err(CliError::NothingToChoose(project.to_path_buf()));
        }
        let items: Vec<(ProcessName, String, String)> = candidates
            .iter()
            .map(|process| {
                (
                    process.name().clone(),
                    process.name().to_string(),
                    format!("{}{HINT_SEPARATOR}{}", process.kind(), process.state()),
                )
            })
            .collect();
        asked(|| select(CHOOSE_PROCESS).items(&items).interact())
    }

    /// Asks `question`, which `argument` answers up front, and answers
    /// whether the person agreed.
    ///
    /// # Errors
    /// Returns [`CliError::CannotAsk`] naming `asked` and `argument` in the
    /// plain layout, or [`CliError::Cancelled`] when the person cancels.
    pub fn confirm(
        self,
        question: &str,
        asked_for: &'static str,
        argument: &'static str,
    ) -> Result<bool, CliError> {
        self.can_confirm(asked_for, argument)?;
        asked(|| confirm(question).interact())
    }

    /// Checks that a yes-or-no question about `asked_for`, which `argument`
    /// answers up front, can be asked, before anything else is done.
    ///
    /// # Errors
    /// Returns [`CliError::CannotAsk`] in the plain layout.
    pub fn can_confirm(
        self,
        asked_for: &'static str,
        argument: &'static str,
    ) -> Result<(), CliError> {
        match self {
            Self::Interactive => Ok(()),
            Self::Plain => Err(CliError::CannotAsk {
                question: asked_for,
                argument,
            }),
        }
    }

    /// The workspace a new project starts with: its processes asked for one
    /// after another at a terminal, the starter workspace otherwise.
    ///
    /// # Errors
    /// Returns [`CliError::Cancelled`] when the person cancels, or the
    /// [`CliError::Config`] of a workspace that does not validate.
    pub fn new_workspace(self) -> Result<WorkspaceConfig, CliError> {
        match self {
            Self::Interactive => {
                let guard = InterruptGuard::arm();
                let workspace = ask_workspace();
                if guard.interrupted() {
                    return Err(CliError::Cancelled);
                }
                workspace
            },
            Self::Plain => Ok(starter_workspace()),
        }
    }
}

/// The answer to the question `ask` asks, with Ctrl-C held as a flag while
/// it is on screen, so one pressed before the prompt reads keys raw cancels
/// it rather than killing the command mid-frame. One already pressed under
/// an enclosing guard cancels before the question is put.
///
/// # Errors
/// Returns [`CliError::Cancelled`] when the person cancelled, or a
/// [`CliError::Terminal`] when the terminal failed.
pub(super) fn asked<T>(ask: impl FnOnce() -> io::Result<T>) -> Result<T, CliError> {
    let guard = InterruptGuard::arm();
    if guard.interrupted() {
        return Err(CliError::Cancelled);
    }
    let answer = ask().map_err(prompt_failure);
    if guard.interrupted() {
        return Err(CliError::Cancelled);
    }
    answer
}

/// The failure of a prompt: cancelled when the person pressed Esc or
/// Ctrl-C, a terminal failure otherwise.
fn prompt_failure(error: io::Error) -> CliError {
    match error.kind() {
        io::ErrorKind::Interrupted => CliError::Cancelled,
        _ => CliError::Terminal(error),
    }
}

/// Writes `text` and a line break to stdout.
///
/// # Errors
/// Returns the [`io::Error`] of a failed write.
fn print_line(text: &str) -> io::Result<()> {
    let mut stdout = anstream::stdout();
    stdout.write_all(format!("{text}\n").as_bytes())?;
    stdout.flush()
}

/// The display width of the widest label among `rows`.
fn label_width(rows: &[Row]) -> usize {
    rows.iter()
        .filter_map(|row| row.label().as_deref().map(UnicodeWidthStr::width))
        .max()
        .unwrap_or(0)
}

/// `row` as a framed line of its kind, its label bold and padded to `width`.
///
/// # Errors
/// Returns the [`io::Error`] of a failed write.
fn framed_row(row: &Row, width: usize) -> io::Result<()> {
    let text = framed_text(row, width);
    match row.kind() {
        RowKind::Ok => log::success(text),
        RowKind::Hint => log::warning(text),
        RowKind::Fail => log::error(text),
        RowKind::Plain => log::info(text),
        RowKind::Dim => log::remark(style(text).dim()),
    }
}

/// The text of `row` in a frame: its bold label padded to `width`, then its
/// detail.
fn framed_text(row: &Row, width: usize) -> String {
    match row.label() {
        Some(label) => {
            let pad = " ".repeat(width - UnicodeWidthStr::width(label.as_str()));
            format!("{}{pad}{LABEL_GAP}{}", style(label).bold(), row.detail())
        },
        None => row.detail().clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::cli::{cli_exit::CliExit, failure_kind::FailureKind};

    /// Labels pad to the widest one so the details line up.
    #[test]
    fn framed_labels_line_up() {
        console::set_colors_enabled(false);
        let rows = [
            Row::labeled(RowKind::Ok, "api", "running"),
            Row::labeled(RowKind::Fail, "dev server", "crashed"),
            Row::unlabeled(RowKind::Dim, "/p/muster.yml"),
        ];
        let width = label_width(&rows);

        assert_eq!(framed_text(&rows[0], width), "api         running");
        assert_eq!(framed_text(&rows[1], width), "dev server  crashed");
        assert_eq!(framed_text(&rows[2], width), "/p/muster.yml");
    }

    /// The plain layout never asks: each question fails naming the argument
    /// that answers it, as invalid input.
    #[test]
    fn the_plain_layout_never_asks() {
        let error = Presenter::Plain
            .confirm(
                "Stop the agents too?",
                "whether to stop the agents",
                "--yes",
            )
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "cannot ask whether to stop the agents without an interactive terminal; pass --yes"
        );
        assert_eq!(error.kind(), FailureKind::InvalidInput);
        assert!(matches!(
            Presenter::Plain.can_choose_process(),
            Err(CliError::CannotAsk {
                argument: CHOOSE_PROCESS_ARGUMENT,
                ..
            })
        ));
        assert!(matches!(
            Presenter::Plain.choose_process(&[], Path::new("/p/muster.yml")),
            Err(CliError::CannotAsk { .. })
        ));
        assert!(Presenter::Interactive.can_choose_process().is_ok());
    }

    /// With nothing to choose from, the question is never put.
    #[test]
    fn nothing_to_choose_is_said_before_asking() {
        assert!(matches!(
            Presenter::Interactive.choose_process(&[], Path::new("/p/muster.yml")),
            Err(CliError::NothingToChoose(_))
        ));
    }

    /// Without a terminal, `muster init` writes the starter workspace.
    #[test]
    fn the_plain_layout_starts_from_the_starter() {
        let workspace = Presenter::Plain.new_workspace().unwrap();
        assert_eq!(
            serde_yaml_ng::to_string(&workspace).unwrap(),
            serde_yaml_ng::to_string(&starter_workspace()).unwrap()
        );
    }

    /// A prompt cancelled with Esc or Ctrl-C is a cancellation, which exits
    /// as a usage error; anything else is the terminal failing.
    #[test]
    fn a_cancelled_prompt_is_a_usage_cancellation() {
        let cancelled = prompt_failure(io::ErrorKind::Interrupted.into());
        assert!(matches!(cancelled, CliError::Cancelled));
        assert_eq!(CliExit::from(cancelled.kind()), CliExit::Usage);
        assert!(matches!(
            prompt_failure(io::ErrorKind::BrokenPipe.into()),
            CliError::Terminal(_)
        ));
    }
}
