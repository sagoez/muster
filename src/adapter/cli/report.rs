use std::{error::Error, io::IsTerminal};

use anstyle::{AnsiColor, Style};
use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;
use unicode_width::UnicodeWidthStr;

/// Style of a passing row's glyph.
const OK_STYLE: Style = AnsiColor::Green.on_default();
/// Style of an advisory row's glyph.
const HINT_STYLE: Style = AnsiColor::Yellow.on_default();
/// Style of a failing row's glyph.
const FAIL_STYLE: Style = AnsiColor::Red.on_default();
/// Style of a row's label column.
const LABEL_STYLE: Style = Style::new().bold();
/// Style of de-emphasized rows and the box border.
const DIM_STYLE: Style = Style::new().dimmed();
/// Style of the box title.
const TITLE_STYLE: Style = Style::new().bold();

/// Glyph of a passing row.
const OK_GLYPH: &str = "●";
/// Glyph of an advisory row.
const HINT_GLYPH: &str = "!";
/// Glyph of a failing row.
const FAIL_GLYPH: &str = "✗";

/// Box-drawing pieces matching the TUI's rounded panes.
const TOP_LEFT: &str = "╭";
const TOP_RIGHT: &str = "╮";
const BOTTOM_LEFT: &str = "╰";
const BOTTOM_RIGHT: &str = "╯";
const VERTICAL: &str = "│";
const HORIZONTAL: &str = "─";
/// Blank columns between the border and a row's content, each side.
const BOX_PADDING: usize = 2;
/// Blank columns between a padded label and its detail.
const COLUMN_GAP: usize = 2;
/// Separator between the parts of a summary line.
const SUMMARY_SEPARATOR: &str = " · ";
/// Separator between a plain row's label and detail.
const PLAIN_SEPARATOR: &str = ": ";

/// How a row is glyphed and colored in the boxed layout. The plain layout
/// shows glyphs only for labeled rows, preserving the pipeable line format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
    /// Healthy: green dot.
    Ok,
    /// Advisory: yellow marker.
    Hint,
    /// Broken: red cross.
    Fail,
    /// Unstyled content.
    Plain,
    /// De-emphasized content (dimmed in the box).
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

    /// The glyph's color style.
    fn style(self) -> Style {
        match self {
            RowKind::Ok => OK_STYLE,
            RowKind::Hint => HINT_STYLE,
            RowKind::Fail => FAIL_STYLE,
            RowKind::Plain => Style::new(),
            RowKind::Dim => DIM_STYLE,
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
    /// An unlabeled row of the given kind.
    pub fn unlabeled(kind: RowKind, detail: impl Into<String>) -> Self {
        Row::builder().kind(kind).detail(detail.into()).build()
    }

    /// A labeled, glyphed row.
    pub fn labeled(kind: RowKind, label: impl Into<String>, detail: impl Into<String>) -> Self {
        Row::builder()
            .kind(kind)
            .label(Some(label.into()))
            .detail(detail.into())
            .build()
    }

    /// The row in the plain line-per-fact layout: labeled rows keep the
    /// `glyph label: detail` shape, unlabeled rows are the bare detail, so
    /// piped output matches the pre-styling format byte for byte.
    fn plain_line(&self) -> String {
        match (&self.label, self.kind.glyph()) {
            (Some(label), Some(glyph)) => {
                format!("{glyph} {label}{PLAIN_SEPARATOR}{}", self.detail)
            },
            _ => self.detail.clone(),
        }
    }
}

/// A command's full output: a titled set of rows and an optional summary
/// shown only in the boxed layout.
#[derive(Clone, Debug, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Report {
    /// Box title, e.g. `muster doctor`.
    title: String,
    /// The facts, in display order.
    rows: Vec<Row>,
    /// Optional closing line, e.g. `5 ok · 1 hint`.
    #[builder(default)]
    summary: Option<String>,
}

impl Report {
    /// A report over `rows` titled `title`.
    pub fn new(title: impl Into<String>, rows: Vec<Row>) -> Self {
        Report::builder().title(title.into()).rows(rows).build()
    }

    /// Attaches the boxed-layout summary line.
    pub fn with_summary(mut self, summary: impl Into<String>) -> Self {
        self.summary = Some(summary.into());
        self
    }
}

/// Joins non-empty summary parts with the standard separator.
pub fn summary_line(parts: &[String]) -> String {
    parts.join(SUMMARY_SEPARATOR)
}

/// Prints a failure to stderr in the CLI's voice: a red cross, the app
/// name, and the error. anstream strips styling when stderr is redirected.
pub fn print_error(context: &str, error: &dyn Error) {
    anstream::eprintln!(
        "{glyph} {context}{PLAIN_SEPARATOR}{error}",
        glyph = paint(FAIL_GLYPH, FAIL_STYLE, true),
    );
}

/// Prints the report: the boxed styled layout on a terminal, the plain
/// line-per-fact layout when piped. anstream strips colors as needed.
pub fn print(report: &Report) {
    if std::io::stdout().is_terminal() {
        anstream::println!("{}", boxed(report, true));
    } else {
        anstream::println!("{}", plain(report));
    }
}

/// The plain layout: one unstyled line per row, matching the CLI's original
/// output exactly.
pub fn plain(report: &Report) -> String {
    report
        .rows
        .iter()
        .map(Row::plain_line)
        .collect::<Vec<_>>()
        .join("\n")
}

/// The boxed layout: a rounded, titled frame around aligned rows, in the
/// TUI's visual vocabulary. `styled` disables ANSI styling for tests.
pub fn boxed(report: &Report, styled: bool) -> String {
    let label_width = report
        .rows
        .iter()
        .filter_map(|row| row.label.as_deref().map(UnicodeWidthStr::width))
        .max()
        .unwrap_or(0);
    let body: Vec<(String, String)> = report
        .rows
        .iter()
        .map(|row| {
            (
                row_content(row, label_width),
                styled_row(row, label_width, styled),
            )
        })
        .chain(
            report
                .summary
                .iter()
                .map(|summary| (summary.clone(), paint(summary, DIM_STYLE, styled))),
        )
        .collect();
    let title_span = UnicodeWidthStr::width(report.title.as_str()) + 2;
    let content_width = body
        .iter()
        .map(|(plain, _)| UnicodeWidthStr::width(plain.as_str()))
        .max()
        .unwrap_or(0)
        .max(title_span)
        + BOX_PADDING * 2;
    let mut lines = Vec::new();
    lines.push(top_border(&report.title, content_width, styled));
    lines.push(spacer_row(content_width, styled));
    for (index, (plain_row, styled_row)) in body.iter().enumerate() {
        let is_summary = report.summary.is_some() && index == body.len() - 1;
        if is_summary {
            lines.push(spacer_row(content_width, styled));
        }
        let fill = content_width - BOX_PADDING - UnicodeWidthStr::width(plain_row.as_str());
        lines.push(format!(
            "{border}{pad}{styled_row}{fill}{border_end}",
            border = paint(VERTICAL, DIM_STYLE, styled),
            pad = " ".repeat(BOX_PADDING),
            fill = " ".repeat(fill),
            border_end = paint(VERTICAL, DIM_STYLE, styled),
        ));
    }
    lines.push(spacer_row(content_width, styled));
    lines.push(bottom_border(content_width, styled));
    lines.join("\n")
}

/// The unstyled content of one boxed row, used for width computation.
fn row_content(row: &Row, label_width: usize) -> String {
    match (&row.label, row.kind.glyph()) {
        (Some(label), Some(glyph)) => format!(
            "{glyph} {label}{pad}{gap}{detail}",
            pad = " ".repeat(label_width - UnicodeWidthStr::width(label.as_str())),
            gap = " ".repeat(COLUMN_GAP),
            detail = row.detail,
        ),
        (Some(label), None) => format!(
            "  {label}{pad}{gap}{detail}",
            pad = " ".repeat(label_width - UnicodeWidthStr::width(label.as_str())),
            gap = " ".repeat(COLUMN_GAP),
            detail = row.detail,
        ),
        (None, Some(glyph)) => format!("{glyph} {}", row.detail),
        (None, None) => row.detail.clone(),
    }
}

/// One boxed row with its glyph colored, label bolded, and dim rows dimmed.
fn styled_row(row: &Row, label_width: usize, styled: bool) -> String {
    if !styled {
        return row_content(row, label_width);
    }
    match (&row.label, row.kind.glyph()) {
        (Some(label), Some(glyph)) => format!(
            "{glyph} {label}{pad}{gap}{detail}",
            glyph = paint(glyph, row.kind.style(), styled),
            label = paint(label, LABEL_STYLE, styled),
            pad = " ".repeat(label_width - UnicodeWidthStr::width(label.as_str())),
            gap = " ".repeat(COLUMN_GAP),
            detail = row.detail,
        ),
        (Some(label), None) => format!(
            "  {label}{pad}{gap}{detail}",
            label = paint(label, LABEL_STYLE, styled),
            pad = " ".repeat(label_width - UnicodeWidthStr::width(label.as_str())),
            gap = " ".repeat(COLUMN_GAP),
            detail = row.detail,
        ),
        (None, Some(glyph)) => format!("{} {}", paint(glyph, row.kind.style(), styled), row.detail),
        (None, None) => match row.kind {
            RowKind::Dim => paint(&row.detail, DIM_STYLE, styled),
            _ => row.detail.clone(),
        },
    }
}

/// Wraps `text` in `style` when styling is on.
fn paint(text: &str, style: Style, styled: bool) -> String {
    if styled {
        format!("{style}{text}{style:#}")
    } else {
        text.to_string()
    }
}

/// The `╭ title ────╮` line.
fn top_border(title: &str, content_width: usize, styled: bool) -> String {
    let used = UnicodeWidthStr::width(title) + 2;
    let fill = HORIZONTAL.repeat(content_width.saturating_sub(used));
    format!(
        "{corner} {title} {fill}{end}",
        corner = paint(TOP_LEFT, DIM_STYLE, styled),
        title = paint(title, TITLE_STYLE, styled),
        fill = paint(&fill, DIM_STYLE, styled),
        end = paint(TOP_RIGHT, DIM_STYLE, styled),
    )
}

/// A `│      │` spacer line.
fn spacer_row(content_width: usize, styled: bool) -> String {
    format!(
        "{border}{space}{border_end}",
        border = paint(VERTICAL, DIM_STYLE, styled),
        space = " ".repeat(content_width),
        border_end = paint(VERTICAL, DIM_STYLE, styled),
    )
}

/// The `╰────╯` closing line.
fn bottom_border(content_width: usize, styled: bool) -> String {
    format!(
        "{corner}{fill}{end}",
        corner = paint(BOTTOM_LEFT, DIM_STYLE, styled),
        fill = paint(&HORIZONTAL.repeat(content_width), DIM_STYLE, styled),
        end = paint(BOTTOM_RIGHT, DIM_STYLE, styled),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Report {
        Report::new("muster doctor", vec![
            Row::labeled(RowKind::Ok, "config", "muster.yml is valid"),
            Row::labeled(RowKind::Hint, "completions", "not registered"),
        ])
        .with_summary("1 ok · 1 hint")
    }

    /// The plain layout reproduces the original line-per-fact output.
    #[test]
    fn plain_layout_matches_the_original_lines() {
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

    /// The box frames aligned rows under the title, closing with the summary.
    #[test]
    fn boxed_layout_frames_and_aligns() {
        let unstyled = boxed(&sample(), false);
        let lines: Vec<&str> = unstyled.lines().collect();
        assert!(lines[0].starts_with("╭ muster doctor "));
        assert!(lines[0].ends_with("╮"));
        assert!(lines[2].contains("● config       muster.yml is valid"));
        assert!(lines[3].contains("! completions  not registered"));
        assert!(lines[5].contains("1 ok · 1 hint"));
        assert!(lines.last().unwrap().starts_with("╰"));
        let widths: Vec<usize> = lines
            .iter()
            .map(|line| UnicodeWidthStr::width(*line))
            .collect();
        assert!(
            widths.windows(2).all(|pair| pair[0] == pair[1]),
            "every box line has the same display width: {widths:?}"
        );
    }

    /// Styled output carries ANSI sequences; unstyled carries none.
    #[test]
    fn styling_toggles_ansi_sequences() {
        assert!(boxed(&sample(), true).contains('\u{1b}'));
        assert!(!boxed(&sample(), false).contains('\u{1b}'));
    }

    /// Summary parts join with the dot separator.
    #[test]
    fn summary_parts_join_with_dots() {
        assert_eq!(
            summary_line(&["5 ok".to_string(), "1 hint".to_string()]),
            "5 ok · 1 hint"
        );
    }
}
