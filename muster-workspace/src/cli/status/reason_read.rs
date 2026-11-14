//! Why a process failed, as one line: the last non-empty line its failed
//! run logged, read through the same rendered read `muster logs -n` uses
//! and bounded in bytes, or the error that kept it from launching.

use std::path::Path;

use console::Term;
use muster_client::{ServiceClient, process};
use muster_domain::{
    project::Project,
    value::{OutputCursor, ProcessName, printable},
};
use muster_protocol::{
    holder::{LineFilter, LineLimit, LogQuery, log_end, read_log},
    service::{ExitReason, Notice, NoticeLevel, ProcessStatus},
};
use unicode_width::UnicodeWidthChar;

use super::failure_reason::FailureReason;

/// How many of a run's newest lines are searched for its last non-empty one.
const REASON_LOOKBACK_LINES: usize = 20;
/// The most of a run's newest output a reason is read from, so a run that
/// ends in megabytes of output that renders to nothing costs no more.
const REASON_WINDOW_BYTES: u64 = 64 * 1024;
/// The width a reason's line is cut to when its stream is no terminal.
const FALLBACK_WIDTH: usize = 100;
/// Ends a reason cut short to fit its line.
const ELLIPSIS: char = '…';
/// The tab a process's line may hold, expanded before it is cut.
const TAB: char = '\t';
/// Columns between tab stops, as a terminal sets them.
const TAB_STOP: usize = 8;

/// Why each of `failed`, processes of `project`, failed, each cut to `width`
/// display columns; a process with no reason is left out. A run that
/// launched gives the last line it logged; one that never launched gives
/// the error the service raised for it.
pub fn failure_reasons<'a>(
    client: &ServiceClient,
    project: &Project,
    failed: impl IntoIterator<Item = &'a ProcessStatus>,
    width: usize,
) -> Vec<FailureReason> {
    let failed: Vec<&ProcessStatus> = failed.into_iter().collect();
    let notices = if failed.iter().any(|process| never_launched(process)) {
        process::service_notices(client, project).unwrap_or_default()
    } else {
        Vec::new()
    };
    failed
        .into_iter()
        .filter_map(|process| {
            let unlaunched = never_launched(process);
            let whole = if unlaunched {
                launch_error(&notices, process.name())
            } else {
                let log = process::log_path(client, project, process.name()).ok()?;
                reason_in(&log, (*process.run_start())?)
            }?;
            Some(
                FailureReason::builder()
                    .root(project.root().clone())
                    .name(process.name().clone())
                    .text(fitted(&whole, width))
                    .whole(whole)
                    .never_launched(unlaunched)
                    .build(),
            )
        })
        .collect()
}

/// A process's `line` fitted to one line of `width` display columns: made
/// printable and its tabs expanded first, so the cut counts every column
/// the line takes and it never wraps to the start of the next.
pub fn fitted(line: &str, width: usize) -> String {
    truncated(&expand_tabs(&printable(line)), width)
}

/// `text` with each tab replaced by the spaces up to the next tab stop.
fn expand_tabs(text: &str) -> String {
    let mut column = 0;
    let mut expanded = String::new();
    for character in text.chars() {
        if character == TAB {
            let spaces = TAB_STOP - column % TAB_STOP;
            expanded.push_str(&" ".repeat(spaces));
            column += spaces;
        } else {
            column += character.width().unwrap_or(0);
            expanded.push(character);
        }
    }
    expanded
}

/// How wide a reason drawn on `term` behind `indent` columns may be: the
/// terminal's width, or [`FALLBACK_WIDTH`] when it is no terminal, less the
/// indent.
pub fn reason_width(term: &Term, indent: usize) -> usize {
    term.size_checked()
        .map_or(FALLBACK_WIDTH, |(_, columns)| usize::from(columns))
        .saturating_sub(indent)
}

/// Whether `process`'s last run never launched, so it logged nothing.
fn never_launched(process: &ProcessStatus) -> bool {
    process
        .last_exit()
        .as_ref()
        .is_some_and(|exit| exit.reason() == ExitReason::FailedToStart)
}

/// The newest error the service raised about `name` among `notices`, made
/// printable.
fn launch_error(notices: &[Notice], name: &ProcessName) -> Option<String> {
    notices
        .iter()
        .rev()
        .find(|notice| {
            notice.process().as_ref() == Some(name) && *notice.level() == NoticeLevel::Error
        })
        .map(|notice| printable(notice.message()))
}

/// The last non-empty line the run that began at `run_start` logged at
/// `log`, read from at most [`REASON_WINDOW_BYTES`] back from the log's
/// end, rendered and made printable; `None` when the run logged nothing or
/// the log cannot be read.
pub fn reason_in(log: &Path, run_start: OutputCursor) -> Option<String> {
    let end = log_end(log).ok()?;
    let from = run_start.max(OutputCursor::new(
        end.offset().saturating_sub(REASON_WINDOW_BYTES),
    ));
    let query = LogQuery::builder()
        .since(Some(from))
        .lines(LineLimit::try_new(REASON_LOOKBACK_LINES).ok()?)
        .filter(LineFilter::All)
        .build();
    let slice = read_log(log, &query).ok()?;
    slice
        .lines()
        .iter()
        .rev()
        .map(|line| printable(line.trim()))
        .find(|line| !line.is_empty())
}

/// `text` cut to `width` display columns, ending in `…` when it was cut.
fn truncated(text: &str, width: usize) -> String {
    let fits = text.chars().map(|c| c.width().unwrap_or(0)).sum::<usize>() <= width;
    if fits {
        return text.to_string();
    }
    let ellipsis_width = ELLIPSIS.width().unwrap_or(1);
    let mut used = 0;
    let mut kept = String::new();
    for character in text.chars() {
        let char_width = character.width().unwrap_or(0);
        if used + char_width + ellipsis_width > width {
            break;
        }
        used += char_width;
        kept.push(character);
    }
    kept.push(ELLIPSIS);
    kept
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::PathBuf,
        time::{Duration, Instant},
    };

    use tempfile::TempDir;

    use super::*;

    /// How long the reason read of a log of many megabytes may take, far
    /// below what reading it whole costs.
    const BOUNDED_READ: Duration = Duration::from_secs(2);
    /// The size of the output that renders to no line after a log's reason.
    const FILLER_BYTES: usize = 32 * 1024 * 1024;

    /// Where a log's output starts.
    fn log_start() -> OutputCursor {
        OutputCursor::new(0)
    }

    /// A log holding `bytes`, in a scratch folder kept alive with it.
    fn log_of(bytes: &[u8]) -> (TempDir, PathBuf) {
        let scratch = TempDir::new().unwrap();
        let log = scratch.path().join("api.log");
        fs::write(&log, bytes).unwrap();
        (scratch, log)
    }

    /// The reason is the last line with text, past trailing blank ones.
    #[test]
    fn the_reason_is_the_last_line_with_text() {
        let (_scratch, log) = log_of(b"starting\nError: port in use\n\n   \n");
        assert_eq!(
            reason_in(&log, log_start()).as_deref(),
            Some("Error: port in use")
        );
    }

    /// Only the failed run's output counts: a run that logged nothing gives
    /// no reason, never the line an earlier run left.
    #[test]
    fn an_earlier_runs_line_is_never_the_reason() {
        let earlier = b"Listening on :3000\n";
        let (_scratch, log) = log_of(earlier);
        let run_start = log_start().advanced(earlier.len() as u64);
        assert_eq!(reason_in(&log, run_start), None);
        assert_eq!(
            reason_in(&log, log_start()).as_deref(),
            Some("Listening on :3000")
        );
    }

    /// A log ending in CSI and OSC sequences gives a reason with no escape
    /// left in it.
    #[test]
    fn a_reason_never_carries_an_escape() {
        let (_scratch, log) =
            log_of(b"\x1b[31mError: boom\x1b[0m\n\x1b]0;pwned\x07\x1b[2J\x1b[H\n\x1b[1A");
        let reason = reason_in(&log, log_start()).unwrap();
        assert!(!reason.chars().any(char::is_control), "{reason:?}");
        assert_eq!(reason, "Error: boom");
    }

    /// The read is bounded in bytes: a run that ends in many megabytes of
    /// carriage returns, or of OSC sequences, which render to no line, is
    /// read only at its tail, so the line before them is out of reach and
    /// the read stays quick.
    #[test]
    fn the_read_is_bounded_in_bytes() {
        for filler in [&b"\r"[..], b"\x1b]0;t\x07"] {
            let mut bytes = b"the real reason\n".to_vec();
            bytes.extend(filler.iter().copied().cycle().take(FILLER_BYTES));
            let (_scratch, log) = log_of(&bytes);
            let started = Instant::now();
            assert_eq!(reason_in(&log, log_start()), None);
            assert!(started.elapsed() < BOUNDED_READ, "{:?}", started.elapsed());
        }
        let (empty_dir, empty) = log_of(b"");
        assert_eq!(reason_in(&empty, log_start()), None);
        assert_eq!(
            reason_in(&empty_dir.path().join("none.log"), log_start()),
            None
        );
    }

    /// Tabs are expanded and controls escaped before the cut, so a fitted
    /// line never takes more columns than its width and cannot wrap.
    #[test]
    fn a_fitted_line_counts_every_column_it_takes() {
        let tabs = format!("start{}X■ db  failed", "\t".repeat(14));
        let fitted_tabs = fitted(&tabs, 40);
        assert!(!fitted_tabs.contains('\t'), "{fitted_tabs:?}");
        assert!(fitted_tabs.chars().count() <= 40, "{fitted_tabs:?}");
        let separators = format!("{}XX■ db  failed", "\u{2028}".repeat(7));
        let fitted_separators = fitted(&separators, 40);
        assert!(
            fitted_separators.chars().count() <= 40,
            "{fitted_separators:?}"
        );
        assert!(
            !fitted_separators.contains('\u{2028}'),
            "{fitted_separators:?}"
        );
    }

    /// A reason too wide for its line ends in `…`, measured by display width.
    #[test]
    fn a_wide_reason_is_cut_with_an_ellipsis() {
        assert_eq!(truncated("short", 10), "short");
        assert_eq!(truncated("Conflict. The name", 10), "Conflict.…");
        assert_eq!(truncated("日本語のテキスト", 7), "日本語…");
    }
}
