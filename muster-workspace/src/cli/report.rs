use std::{
    error::Error,
    io::{self, Write},
};

#[cfg(test)]
use anstream::adapter::strip_str;
use getset::{CopyGetters, Getters};
use muster_domain::{coordination::Author, value::printable};
use muster_os::sanitize::printable_lines;
use typed_builder::TypedBuilder;
use unicode_width::UnicodeWidthStr;

use super::{
    row::Row,
    row_kind::{RowKind, dimmed},
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

/// Separator between the parts of a summary line.
pub(super) const SUMMARY_SEPARATOR: &str = " · ";
/// Between a mark and its line.
const MARK_GAP: &str = " ";
/// Between a padded label and its detail.
pub(super) const LABEL_GAP: &str = "  ";
/// Indent of a dimmed detail line under its mark.
pub(super) const NOTE_INDENT: &str = "  ";
/// Summary word for passing rows.
const SUMMARY_OK: &str = "ok";
/// Summary words for advisory rows.
const SUMMARY_HINT: &str = "hint";
const SUMMARY_HINTS: &str = "hints";
/// Summary word for failing rows, the word `ps` counts failed processes by.
const SUMMARY_FAILED: &str = "failed";
/// Between a problem's head and its hint on one line of an error message.
const HINT_SEPARATOR: &str = "; ";
/// The quotes a name in an error message is wrapped in, inside which a
/// [`HINT_SEPARATOR`] belongs to the name.
const QUOTES: [char; 2] = ['\'', '"'];

/// A command's full output: a titled set of rows, an optional summary the
/// interactive layout closes with, what to run next, and whether the
/// command failed.
#[derive(Clone, Debug, Getters, CopyGetters, TypedBuilder)]
pub struct Report {
    /// Opening title, e.g. `muster doctor`.
    #[getset(get = "pub")]
    title: String,
    /// The facts, in display order.
    #[getset(get = "pub")]
    rows: Vec<Row>,
    /// Optional closing line, e.g. `1 failed · 5 ok`.
    #[builder(default)]
    #[getset(get = "pub")]
    summary: Option<String>,
    /// What to run next, such as `Run muster logs api to see why it failed.`
    #[builder(default)]
    #[getset(get = "pub")]
    next_step: Option<String>,
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

    /// Attaches what to run next, made [`printable`], when there is
    /// something to run.
    pub fn with_next_step(mut self, next_step: Option<String>) -> Self {
        self.next_step = next_step.map(|step| printable(&step));
        self
    }

    /// Marks whether the command `failed`.
    pub fn with_failure(mut self, failed: bool) -> Self {
        self.failed = failed;
        self
    }
}

/// A summary counting the labeled `rows` by kind, failed first and zero
/// counts left out, such as `1 failed · 2 ok`, or `None` when no row is
/// labeled.
pub fn tally(rows: &[Row]) -> Option<String> {
    let count = |kind: RowKind| {
        rows.iter()
            .filter(|row| row.label().is_some() && row.kind() == kind)
            .count()
    };
    let hints = count(RowKind::Hint);
    let parts: Vec<String> = [
        (count(RowKind::Fail), SUMMARY_FAILED),
        (count(RowKind::Ok), SUMMARY_OK),
        (hints, plural(hints, SUMMARY_HINT, SUMMARY_HINTS)),
    ]
    .into_iter()
    .filter(|(count, _)| *count > 0)
    .map(|(count, word)| format!("{count} {word}"))
    .collect();
    (!parts.is_empty()).then(|| parts.join(SUMMARY_SEPARATOR))
}

/// The singular or plural word for a count.
pub(super) fn plural<'a>(count: usize, singular: &'a str, many: &'a str) -> &'a str {
    if count == 1 { singular } else { many }
}

/// A problem's message split into its head and the hint lines under it:
/// the first line up to its first `; ` outside quotes, then the rest of that
/// line and every later line.
pub fn problem_lines(message: &str) -> (String, Vec<String>) {
    let mut lines = message.lines();
    let first = lines.next().unwrap_or_default();
    let (head, hint) = split_hint(first);
    let details = hint
        .into_iter()
        .chain(lines.map(str::to_string))
        .filter(|line| !line.trim().is_empty())
        .collect();
    (head, details)
}

/// `line` split at its first [`HINT_SEPARATOR`] outside quotes, so a quoted
/// name holding one stays whole.
fn split_hint(line: &str) -> (String, Option<String>) {
    let mut open: Option<char> = None;
    for (at, character) in line.char_indices() {
        match open {
            Some(quote) if character == quote => open = None,
            Some(_) => {},
            None if QUOTES.contains(&character) => open = Some(character),
            None if line[at..].starts_with(HINT_SEPARATOR) => {
                return (
                    line[..at].to_string(),
                    Some(line[at + HINT_SEPARATOR.len()..].to_string()),
                );
            },
            None => {},
        }
    }
    (line.to_string(), None)
}

/// `error` as the lines a problem shows: a `■` head, then its hint lines
/// dimmed under it, each made printable.
pub fn problem_text(error: &dyn Error) -> String {
    let (head, details) = problem_lines(&printable_lines(&error.to_string()));
    let mut text = head;
    for detail in details {
        text.push('\n');
        text.push_str(&dimmed(&detail));
    }
    text
}

/// Prints a problem to stderr: a red `■`, its head, and its hint dimmed
/// under it, made printable line by line. anstream strips styling where
/// stderr cannot show it.
pub fn print_error(error: &dyn Error) {
    let (head, details) = problem_lines(&printable_lines(&error.to_string()));
    let row = details
        .into_iter()
        .fold(Row::unlabeled(RowKind::Fail, head), Row::with_note);
    anstream::eprintln!("{}", plain_line(&row, 0));
}

/// Prints a warning to stderr, made printable, so a command whose output is
/// data still says something the person should know.
pub fn print_note(note: &str) {
    anstream::eprintln!("{}", plain_line(&Row::unlabeled(RowKind::Hint, note), 0));
}

/// The display width of the widest label among `rows`.
pub(super) fn label_width(rows: &[Row]) -> usize {
    rows.iter()
        .filter_map(|row| row.label().as_deref().map(UnicodeWidthStr::width))
        .max()
        .unwrap_or(0)
}

/// `row`'s label padded to `width`, then its detail; just the detail
/// without a label.
pub(super) fn labeled_text(row: &Row, width: usize) -> String {
    match row.label() {
        Some(label) => {
            let pad = " ".repeat(width.saturating_sub(UnicodeWidthStr::width(label.as_str())));
            format!("{label}{pad}{LABEL_GAP}{}", row.detail())
                .trim_end()
                .to_string()
        },
        None => row.detail().clone(),
    }
}

/// `row` as styled plain lines: its coloured mark, its label padded to
/// `width` and its detail, then each note dimmed and indented under it. An
/// unmarked dim row is dimmed whole.
pub fn plain_line(row: &Row, width: usize) -> String {
    let text = labeled_text(row, width);
    let mut line = match (row.kind().styled_mark(), row.kind()) {
        (Some(mark), _) => format!("{mark}{MARK_GAP}{text}"),
        (None, RowKind::Dim) => dimmed(&text),
        (None, _) => text,
    };
    for note in row.notes() {
        line.push_str(&format!("\n{NOTE_INDENT}{}", dimmed(note)));
    }
    line
}

/// Writes `report`'s rows as styled plain lines, then what to run next: with
/// `split`, warnings and problems to `err` and everything else to `out`;
/// otherwise all of it to `out`, as data a command answers with is.
///
/// # Errors
/// Returns the [`io::Error`] of a failed write.
pub fn write_plain(
    report: &Report,
    out: &mut dyn Write,
    err: &mut dyn Write,
    split: bool,
) -> io::Result<()> {
    let width = label_width(report.rows());
    for row in report.rows() {
        let stream: &mut dyn Write = if split && row.kind().on_stderr() {
            &mut *err
        } else {
            &mut *out
        };
        writeln!(stream, "{}", plain_line(row, width))?;
        stream.flush()?;
    }
    if let Some(step) = report.next_step() {
        writeln!(out, "{step}")?;
    }
    out.flush()
}

/// The plain layout as one unstyled text, every line in order, as a pipe
/// joining both streams shows it; what the tests compare.
#[cfg(test)]
pub fn plain(report: &Report) -> String {
    let mut text = Vec::new();
    let mut sink = io::sink();
    // Writing to memory cannot fail.
    let _ = write_plain(report, &mut text, &mut sink, false);
    strip_str(&String::from_utf8_lossy(&text))
        .to_string()
        .trim_end()
        .to_string()
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

    /// A report with a row of every kind.
    fn sample() -> Report {
        Report::new("muster doctor", vec![
            Row::labeled(RowKind::Ok, "config", "muster.yml is valid"),
            Row::labeled(RowKind::Hint, "completions", "not registered")
                .with_note("muster completions bash"),
        ])
        .with_next_step(Some("Run muster doctor again.".to_string()))
    }

    /// `report` written plain to two buffers, stdout then stderr, coloured or
    /// not.
    fn streams(report: &Report, colour: bool) -> (String, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        write_plain(report, &mut out, &mut err, true).unwrap();
        let shown = |bytes: Vec<u8>| {
            let text = String::from_utf8(bytes).unwrap();
            if colour {
                text
            } else {
                strip_str(&text).to_string()
            }
        };
        (shown(out), shown(err))
    }

    /// The plain layout is one line per fact, marked by kind, labels padded,
    /// notes indented under their mark and the next step last.
    #[test]
    fn plain_layout_is_one_line_per_fact() {
        assert_eq!(
            plain(&sample()),
            "◇ config       muster.yml is valid\n▲ completions  not registered\n  muster \
             completions bash\nRun muster doctor again."
        );
        let bare = Report::new("muster init", vec![
            Row::unlabeled(RowKind::Ok, "created muster.yml"),
            Row::unlabeled(RowKind::Dim, "run `muster` here to start"),
        ]);
        assert_eq!(
            plain(&bare),
            "◇ created muster.yml\nrun `muster` here to start"
        );
    }

    /// `◇`, `○` and unmarked lines go to stdout; `▲` and `■` lines, with
    /// their notes, go to stderr.
    #[test]
    fn each_mark_goes_to_its_stream() {
        let report = Report::new("muster up", vec![
            Row::unlabeled(RowKind::Dim, "/p"),
            Row::labeled(RowKind::Ok, "a", "running"),
            Row::labeled(RowKind::Idle, "b", "stopped"),
            Row::labeled(RowKind::Hint, "c", "restarting"),
            Row::labeled(RowKind::Fail, "d", "failed").with_note("boom"),
        ])
        .with_next_step(Some("Run muster logs d to see why it failed.".to_string()));
        let (out, err) = streams(&report, false);
        assert_eq!(
            out,
            "/p\n◇ a  running\n○ b  stopped\nRun muster logs d to see why it failed.\n"
        );
        assert_eq!(err, "▲ c  restarting\n■ d  failed\n  boom\n");
    }

    /// Coloured, only the marks carry colour and the detail is dimmed; piped,
    /// the same words carry none.
    #[test]
    fn colour_is_for_the_marks_and_dim_detail_only() {
        let report = Report::new("muster up", vec![
            Row::labeled(RowKind::Fail, "infra", "failed  exit 1").with_note("Conflict."),
        ]);
        let (_, coloured) = streams(&report, true);
        assert_eq!(
            coloured,
            "\u{1b}[31m■\u{1b}[0m infra  failed  exit 1\n  \u{1b}[2mConflict.\u{1b}[0m\n"
        );
        let (_, piped) = streams(&report, false);
        assert_eq!(piped, "■ infra  failed  exit 1\n  Conflict.\n");
        insta::assert_snapshot!("notice_piped", piped);
        insta::assert_snapshot!("notice_coloured", format!("{coloured:?}"));
    }

    /// The tally counts labeled rows by kind, failed first and zero counts
    /// left out, and is absent without any.
    #[test]
    fn the_tally_counts_labeled_rows() {
        assert_eq!(
            sample().with_tally().summary().as_deref(),
            Some("1 ok · 1 hint")
        );
        let failing = Report::new("muster up", vec![
            Row::unlabeled(RowKind::Dim, "/p/muster.yml"),
            Row::labeled(RowKind::Fail, "a", "failed"),
            Row::labeled(RowKind::Ok, "b", "running"),
        ]);
        assert_eq!(
            failing.with_tally().summary().as_deref(),
            Some("1 failed · 1 ok")
        );
        let unlabeled = Report::new("muster init", vec![Row::unlabeled(RowKind::Ok, "created")]);
        assert_eq!(unlabeled.with_tally().summary(), &None);
    }

    /// A problem's hint goes under its head, and a quoted name holding the
    /// separator stays whole.
    #[test]
    fn a_problem_heads_and_its_hint_goes_under() {
        assert_eq!(
            problem_lines("no registered project is named 'x'; `muster projects` lists them"),
            ("no registered project is named 'x'".to_string(), vec![
                "`muster projects` lists them".to_string()
            ])
        );
        assert_eq!(
            problem_lines("no process 'a; b' in shop; valid names: 'c'"),
            ("no process 'a; b' in shop".to_string(), vec![
                "valid names: 'c'".to_string()
            ])
        );
        assert_eq!(
            problem_lines("could not read /p"),
            ("could not read /p".to_string(), Vec::new())
        );
    }

    /// A row or title carrying a process's escape sequence shows it as
    /// text, in either layout.
    #[test]
    fn rows_and_titles_never_carry_control_characters() {
        let report = Report::new("muster up \u{b7} \u{1b}[2J", vec![
            Row::labeled(RowKind::Fail, "a\u{1b}]0;x\u{7}", "failed\r").with_note("\u{1b}[31m"),
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
