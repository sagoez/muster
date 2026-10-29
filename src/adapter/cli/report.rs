use std::error::Error;

use anstyle::{AnsiColor, Style};
use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use crate::{
    adapter::sanitize::{printable, printable_lines},
    domain::coordination::Author,
};

/// Columns before an author attribution suffix on a listed line.
const ATTRIBUTION_GAP: &str = "  ";

/// A trailing `(by <name>)` attribution for a coordination entry, or an empty
/// string when the human wrote it - so a human's own lists stay uncluttered and
/// an agent's contributions stand out.
pub(crate) fn attribution(author: &Author) -> String {
    if author.is_human() {
        String::new()
    } else {
        format!("{ATTRIBUTION_GAP}(by {author})")
    }
}

/// Style of the glyph that opens a failure on stderr.
const FAIL_STYLE: Style = AnsiColor::Red.on_default();

/// Glyph of a passing row.
pub(super) const OK_GLYPH: &str = "●";
/// Glyph of an advisory row.
pub(super) const HINT_GLYPH: &str = "!";
/// Glyph of a failing row.
pub(super) const FAIL_GLYPH: &str = "✗";

/// Separator between the parts of a summary line.
const SUMMARY_SEPARATOR: &str = " · ";
/// Separator between a plain row's label and detail.
const PLAIN_SEPARATOR: &str = ": ";
/// Summary word for passing rows.
const SUMMARY_OK: &str = "ok";
/// Summary words for advisory rows.
const SUMMARY_HINT: &str = "hint";
const SUMMARY_HINTS: &str = "hints";
/// Summary words for failing rows.
const SUMMARY_FAILURE: &str = "failure";
const SUMMARY_FAILURES: &str = "failures";

/// What a row says about its fact: how it is glyphed in the plain layout and
/// which kind of line it becomes in the interactive one. The plain layout
/// shows glyphs only for labeled rows, preserving the pipeable line format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    /// Healthy.
    Ok,
    /// Advisory.
    Hint,
    /// Broken.
    Fail,
    /// Unstyled content.
    Plain,
    /// De-emphasized content.
    Dim,
}

impl RowKind {
    /// The status glyph for glyphed kinds.
    fn glyph(self) -> Option<&'static str> {
        match self {
            RowKind::Ok => Some(OK_GLYPH),
            RowKind::Hint => Some(HINT_GLYPH),
            RowKind::Fail => Some(FAIL_GLYPH),
            RowKind::Plain | RowKind::Dim => None,
        }
    }
}

/// One line of command output: an optional labeled fact with a status kind.
#[derive(Clone, Debug, Getters, CopyGetters, TypedBuilder)]
pub struct Row {
    /// Status kind driving glyph and color.
    #[getset(get_copy = "pub")]
    kind: RowKind,
    /// Optional bold label column.
    #[builder(default)]
    #[getset(get = "pub")]
    label: Option<String>,
    /// The fact itself.
    #[getset(get = "pub")]
    detail: String,
}

impl Row {
    /// An unlabeled row of the given kind, its detail made [`printable`].
    pub fn unlabeled(kind: RowKind, detail: impl Into<String>) -> Self {
        Row::builder()
            .kind(kind)
            .detail(printable(&detail.into()))
            .build()
    }

    /// A labeled, glyphed row, its label and detail made [`printable`].
    pub fn labeled(kind: RowKind, label: impl Into<String>, detail: impl Into<String>) -> Self {
        Row::builder()
            .kind(kind)
            .label(Some(printable(&label.into())))
            .detail(printable(&detail.into()))
            .build()
    }

    /// The row in the plain line-per-fact layout: labeled rows keep the
    /// `glyph label: detail` shape, unlabeled rows are the bare detail, so
    /// piped output keeps one stable format.
    fn plain_line(&self) -> String {
        match (&self.label, self.kind.glyph()) {
            (Some(label), Some(glyph)) => {
                format!("{glyph} {label}{PLAIN_SEPARATOR}{}", self.detail)
            },
            _ => self.detail.clone(),
        }
    }
}

/// A command's full output: a titled set of rows, an optional summary the
/// interactive layout closes with, and whether the command failed.
#[derive(Clone, Debug, Getters, CopyGetters, TypedBuilder)]
pub struct Report {
    /// Opening title, e.g. `muster doctor`.
    #[getset(get = "pub")]
    title: String,
    /// The facts, in display order.
    #[getset(get = "pub")]
    rows: Vec<Row>,
    /// Optional closing line, e.g. `5 ok · 1 hint`.
    #[builder(default)]
    #[getset(get = "pub")]
    summary: Option<String>,
    /// Whether the command failed, which closes the interactive layout as a
    /// failure.
    #[builder(default)]
    #[getset(get_copy = "pub")]
    failed: bool,
}

impl Report {
    /// A report over `rows` titled `title`, made [`printable`].
    pub fn new(title: impl Into<String>, rows: Vec<Row>) -> Self {
        Report::builder()
            .title(printable(&title.into()))
            .rows(rows)
            .build()
    }

    /// Attaches the summary line.
    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }

    /// Attaches the [`tally`] of its rows as the summary, when there is one.
    pub fn with_tally(mut self) -> Self {
        self.summary = tally(&self.rows).or(self.summary);
        self
    }

    /// Marks whether the command `failed`.
    pub fn with_failure(mut self, failed: bool) -> Self {
        self.failed = failed;
        self
    }
}

/// A summary counting the labeled `rows` by kind, such as `2 ok · 1 hint`,
/// or `None` when no row is labeled.
pub fn tally(rows: &[Row]) -> Option<String> {
    let count = |kind: RowKind| {
        rows.iter()
            .filter(|row| row.label.is_some() && row.kind == kind)
            .count()
    };
    let (ok, hints, failures) = (
        count(RowKind::Ok),
        count(RowKind::Hint),
        count(RowKind::Fail),
    );
    if ok + hints + failures == 0 {
        return None;
    }
    let mut parts = vec![format!("{ok} {SUMMARY_OK}")];
    if hints > 0 {
        parts.push(format!(
            "{hints} {}",
            plural(hints, SUMMARY_HINT, SUMMARY_HINTS)
        ));
    }
    if failures > 0 {
        parts.push(format!(
            "{failures} {}",
            plural(failures, SUMMARY_FAILURE, SUMMARY_FAILURES)
        ));
    }
    Some(parts.join(SUMMARY_SEPARATOR))
}

/// The singular or plural word for a count.
fn plural<'a>(count: usize, singular: &'a str, many: &'a str) -> &'a str {
    if count == 1 { singular } else { many }
}

/// Prints a failure to stderr in the CLI's voice: a red cross, the app
/// name, and the error, made printable line by line. anstream strips styling
/// when stderr is redirected.
pub fn print_error(context: &str, error: &dyn Error) {
    let message = printable_lines(&error.to_string());
    anstream::eprintln!(
        "{FAIL_STYLE}{FAIL_GLYPH}{FAIL_STYLE:#} {context}{PLAIN_SEPARATOR}{message}"
    );
}

/// The plain layout: one unstyled line per row.
pub fn plain(report: &Report) -> String {
    report
        .rows
        .iter()
        .map(Row::plain_line)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The human writer gets no attribution; an agent gets a `(by name)` suffix.
    #[test]
    fn attribution_marks_only_agents() {
        assert_eq!(attribution(&Author::human()), "");
        assert_eq!(attribution(&Author::agent("claude")), "  (by claude)");
    }

    fn sample() -> Report {
        Report::new("muster doctor", vec![
            Row::labeled(RowKind::Ok, "config", "muster.yml is valid"),
            Row::labeled(RowKind::Hint, "completions", "not registered"),
        ])
    }

    /// The plain layout is one line per fact, glyphed only when labeled.
    #[test]
    fn plain_layout_is_one_line_per_fact() {
        assert_eq!(
            plain(&sample()),
            "● config: muster.yml is valid\n! completions: not registered"
        );
        let bare = Report::new("muster init", vec![
            Row::unlabeled(RowKind::Ok, "created muster.yml"),
            Row::unlabeled(RowKind::Dim, "run `muster` here to start"),
        ]);
        assert_eq!(
            plain(&bare),
            "created muster.yml\nrun `muster` here to start",
            "unlabeled rows never gain glyphs in the plain layout"
        );
    }

    /// The tally counts labeled rows by kind and is absent without any.
    #[test]
    fn the_tally_counts_labeled_rows() {
        assert_eq!(
            sample().with_tally().summary().as_deref(),
            Some("1 ok · 1 hint")
        );
        let failing = Report::new("muster up", vec![
            Row::unlabeled(RowKind::Dim, "/p/muster.yml"),
            Row::labeled(RowKind::Fail, "a", "crashed"),
            Row::labeled(RowKind::Fail, "b", "crashed"),
        ]);
        assert_eq!(
            failing.with_tally().summary().as_deref(),
            Some("0 ok · 2 failures")
        );
        let unlabeled = Report::new("muster init", vec![Row::unlabeled(RowKind::Ok, "created")]);
        assert_eq!(unlabeled.with_tally().summary(), &None);
    }

    /// A row or title carrying a process's escape sequence shows it as
    /// text, in either layout.
    #[test]
    fn rows_and_titles_never_carry_control_characters() {
        let report = Report::new("muster up \u{b7} \u{1b}[2J", vec![
            Row::labeled(RowKind::Fail, "a\u{1b}]0;x\u{7}", "crashed\r"),
            Row::unlabeled(RowKind::Dim, "\u{1b}[31m"),
        ]);

        assert!(!report.title().chars().any(char::is_control));
        assert!(
            !plain(&report)
                .replace('\n', "")
                .chars()
                .any(char::is_control)
        );
    }
}
