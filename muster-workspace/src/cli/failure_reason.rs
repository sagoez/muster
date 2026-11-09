//! Why a process failed, as one line: the last non-empty line of its log,
//! read through the same rendered, bounded read `muster logs -n` uses.

use std::path::Path;

use console::Term;
use muster_client::{ServiceClient, process};
use muster_domain::{
    project::Project,
    value::{ProcessName, printable},
};
use muster_protocol::holder::{LineFilter, LineLimit, LogQuery, read_log};
use unicode_width::UnicodeWidthChar;

use super::report::NOTE_INDENT;

/// How many of a log's newest lines are searched for its last non-empty one.
const REASON_LOOKBACK_LINES: usize = 20;
/// The width a reason's line is cut to when stdout is no terminal.
const FALLBACK_WIDTH: usize = 100;
/// Ends a reason cut short to fit its line.
const ELLIPSIS: char = '…';

/// Why each of `names` of `project` failed, by name, each cut to fit its
/// dimmed line under its mark; a process whose log has no reason is left
/// out.
pub fn failure_reasons<'a>(
    client: &ServiceClient,
    project: &Project,
    names: impl IntoIterator<Item = &'a ProcessName>,
) -> Vec<(ProcessName, String)> {
    let width = line_width().saturating_sub(NOTE_INDENT.len());
    names
        .into_iter()
        .filter_map(|name| {
            failure_reason(client, project, name)
                .map(|reason| (name.clone(), truncated(&reason, width)))
        })
        .collect()
}

/// The display width of stdout's lines: the terminal's, or
/// [`FALLBACK_WIDTH`] when stdout is no terminal.
fn line_width() -> usize {
    Term::stdout()
        .size_checked()
        .map_or(FALLBACK_WIDTH, |(_, columns)| usize::from(columns))
}

/// Why `name` of `project` failed: the last non-empty line of its log, or
/// `None` when there is none or the log cannot be read.
fn failure_reason(client: &ServiceClient, project: &Project, name: &ProcessName) -> Option<String> {
    let log = process::log_path(client, project, name).ok()?;
    reason_in(&log)
}

/// The last non-empty line of the newest [`REASON_LOOKBACK_LINES`] of the
/// log at `log`, rendered and made printable, or `None` when there is none
/// or it cannot be read.
pub fn reason_in(log: &Path) -> Option<String> {
    let query = LogQuery::builder()
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
pub fn truncated(text: &str, width: usize) -> String {
    let mut used = 0;
    let mut kept = String::new();
    let ellipsis_width = ELLIPSIS.width().unwrap_or(1);
    let fits = text.chars().map(|c| c.width().unwrap_or(0)).sum::<usize>() <= width;
    if fits {
        return text.to_string();
    }
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
    use std::{fs, path::PathBuf};

    use tempfile::TempDir;

    use super::*;

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
        assert_eq!(reason_in(&log).as_deref(), Some("Error: port in use"));
    }

    /// A log ending in CSI and OSC sequences gives a reason with no escape
    /// left in it.
    #[test]
    fn a_reason_never_carries_an_escape() {
        let (_scratch, log) =
            log_of(b"\x1b[31mError: boom\x1b[0m\n\x1b]0;pwned\x07\x1b[2J\x1b[H\n\x1b[1A");
        let reason = reason_in(&log).unwrap();
        assert!(!reason.chars().any(char::is_control), "{reason:?}");
        assert_eq!(reason, "Error: boom");
    }

    /// The read is bounded: a long log is read only at its tail, and a
    /// missing or empty log gives no reason rather than an error.
    #[test]
    fn the_read_is_bounded_and_quiet_when_there_is_nothing() {
        let mut long = Vec::new();
        for line in 0..10_000 {
            long.extend_from_slice(format!("line {line}\n").as_bytes());
        }
        let (_scratch, log) = log_of(&long);
        assert_eq!(reason_in(&log).as_deref(), Some("line 9999"));
        let (empty_dir, empty) = log_of(b"");
        assert_eq!(reason_in(&empty), None);
        assert_eq!(reason_in(&empty_dir.path().join("none.log")), None);
    }

    /// A reason too wide for its line ends in `…`, measured by display width.
    #[test]
    fn a_wide_reason_is_cut_with_an_ellipsis() {
        assert_eq!(truncated("short", 10), "short");
        assert_eq!(truncated("Conflict. The name", 10), "Conflict.…");
        assert_eq!(truncated("日本語のテキスト", 7), "日本語…");
    }
}
