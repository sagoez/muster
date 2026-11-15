use muster_client::LogRead;
use muster_domain::value::printable;
use muster_protocol::holder::{HolderError, read_log_tail};

use crate::floor::LocatedLog;

/// Most bytes a card's log tail reads off the end of a log.
pub const TAIL_BYTES: u64 = 16 * 1024;
/// Most lines a card's log tail shows.
pub const TAIL_LINES: usize = 12;
/// What ends a rendered line.
const LINE_BREAK: char = '\n';

/// The last [`TAIL_LINES`] lines of `log` that show anything, read from at
/// most its last [`TAIL_BYTES`] and from a line boundary, rendered as
/// `muster logs` prints a child's output, without color, and made printable
/// as the CLI makes it; none when it has no log yet.
///
/// # Errors
/// Returns [`HolderError::Io`] when the log cannot be read.
pub fn read_tail(log: &LocatedLog) -> Result<Vec<String>, HolderError> {
    let (_, bytes) = read_log_tail(log.path(), TAIL_BYTES)?;
    let read = LogRead::builder()
        .name(log.name().clone())
        .log(Some(log.path().clone()))
        .end(None)
        .bytes(bytes)
        .build();
    let text = read.text_in(log.project());
    let shown: Vec<String> = text
        .text()
        .split(LINE_BREAK)
        .filter(|line| !line.trim().is_empty())
        .map(printable)
        .collect();
    let skipped = shown.len().saturating_sub(TAIL_LINES);
    Ok(shown.into_iter().skip(skipped).collect())
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use muster_domain::value::ProcessName;
    use muster_os::test_scratch::short_scratch;

    use super::*;
    use crate::floor::fixture::project;

    /// Bytes in each old line of the long log.
    const OLD_LINE: usize = 1024;
    /// Bytes in each new line of the long log.
    const NEW_LINE: usize = 4096;
    /// The long log's size.
    const LONG_LOG: usize = 64 * 1024;

    /// Shop's `web`, logging at `path`.
    fn located(path: PathBuf) -> LocatedLog {
        LocatedLog::builder()
            .project(project("shop"))
            .name(ProcessName::try_new("web").unwrap())
            .path(path)
            .build()
    }

    /// `label` padded with dots to `width` bytes, a line break included.
    fn padded(label: &str, width: usize) -> String {
        format!("{label}{}\n", ".".repeat(width - label.len() - 1))
    }

    /// Of 20 lines, the tail keeps the last 12 that show anything, a
    /// colored line as its text and a bell line with nothing that steers a
    /// terminal.
    #[test]
    fn the_tail_keeps_the_last_twelve_printable_lines() {
        let scratch = short_scratch("mv-");
        let path = scratch.path().join("web.log");
        let mut log = String::new();
        for line in 1..=18 {
            log.push_str(&format!("line {line}\n\n"));
        }
        log.push_str("\x1b[31mred\x1b[0m\n");
        log.push_str("ding\x07\x1b]0;title\x07\n");
        fs::write(&path, log).unwrap();

        let lines = read_tail(&located(path)).unwrap();

        assert_eq!(lines.len(), TAIL_LINES, "{lines:?}");
        assert_eq!(lines[0], "line 9");
        assert_eq!(lines[10], "red");
        assert!(lines[11].starts_with("ding"), "{lines:?}");
        assert!(
            lines
                .iter()
                .flat_map(|line| line.chars())
                .all(|character| !character.is_control()),
            "{lines:?}"
        );
    }

    /// Of a 64 KiB log, the tail reads only the last 16 KiB: none of the
    /// lines before them shows, though fewer than 12 lines fit in them.
    #[test]
    fn the_tail_reads_at_most_sixteen_kib() {
        let scratch = short_scratch("mv-");
        let path = scratch.path().join("web.log");
        let old = (LONG_LOG - TAIL_BYTES as usize) / OLD_LINE;
        let new = TAIL_BYTES as usize / NEW_LINE;
        let mut log = String::new();
        for line in 0..old {
            log.push_str(&padded(&format!("old {line}"), OLD_LINE));
        }
        for line in 0..new {
            log.push_str(&padded(&format!("new {line}"), NEW_LINE));
        }
        assert_eq!(log.len(), LONG_LOG);
        fs::write(&path, log).unwrap();

        let lines = read_tail(&located(path)).unwrap();

        assert!(!lines.is_empty());
        assert!(lines.len() <= new, "{lines:?}");
        assert!(
            lines.iter().all(|line| line.starts_with("new ")),
            "{lines:?}"
        );
    }

    /// A process that has logged nothing yet has no lines.
    #[test]
    fn no_log_yet_is_no_lines() {
        let scratch = short_scratch("mv-");

        let lines = read_tail(&located(scratch.path().join("web.log"))).unwrap();

        assert!(lines.is_empty(), "{lines:?}");
    }
}
