//! Turning one raw output line into the plain text an agent reads, through
//! the same rendering `muster logs` shows: escape sequences removed,
//! carriage returns and erases applied, so a progress bar collapses to its
//! last state, and a line that only redrew the screen dropped.

use crate::adapter::terminal::{log_line::LogLine, log_renderer::LogRenderer};

/// Longest a line of output grows without a newline before it is cut into a
/// line of its own, so a stream with no newlines never grows one line
/// without bound.
pub const LINE_MAX_BYTES: usize = 64 * 1024;
/// Ends a line.
const LINE_FEED: u8 = b'\n';

/// The text of one raw output line, its line feed there or not, without
/// that line feed or trailing whitespace, invalid UTF-8 replaced; none when
/// the line only redrew the screen, such as a status footer or the room
/// made for it.
pub fn strip_line(raw: &[u8]) -> Option<String> {
    let mut renderer = LogRenderer::new();
    renderer.advance(raw);
    if !raw.ends_with(&[LINE_FEED]) {
        renderer.advance(&[LINE_FEED]);
    }
    let lines = renderer.take_lines();
    (!lines.is_empty()).then(|| lines.iter().map(LogLine::plain).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What `raw` strips to, a line that only redrew read as empty.
    fn stripped(raw: &[u8]) -> String {
        strip_line(raw).unwrap_or_default()
    }

    #[test]
    fn escape_sequences_are_removed() {
        assert_eq!(stripped(b"\x1b[1;32mready\x1b[0m in 3ms\r"), "ready in 3ms");
        assert_eq!(
            stripped(b"\x1b]0;title\x07Listening on :3000"),
            "Listening on :3000"
        );
        assert_eq!(stripped(b"bell\x07 and \x1b(0line"), "bell and line");
    }

    /// A progress bar redrawn with carriage returns reads as its last state.
    #[test]
    fn carriage_returns_overwrite() {
        assert_eq!(stripped(b"10%\r50%\r100% done"), "100% done");
        assert_eq!(stripped(b"downloading 1/3\r\x1b[Kok"), "ok");
        assert_eq!(stripped(b"abcdef\rXY"), "XYcdef");
        assert_eq!(stripped(b"line\r\n"), "line");
        assert_eq!(stripped(b"abc\x08\x08Z"), "aZc");
        assert_eq!(stripped(b"abcdef\rabc\x1b[1K!"), "   !ef");
    }

    /// A spinner that returns with cursor-to-column redraws in place, and a
    /// column far past the line is clamped rather than padded without end.
    #[test]
    fn cursor_to_column_moves_within_the_line() {
        assert_eq!(stripped(b"\x1b[1G- working\x1b[1G\x1b[0Kdone"), "done");
        assert_eq!(stripped(b"ab\x1b[5Gc"), "ab  c");
        assert_eq!(stripped(b"\x1b[65535Gx").chars().count(), 1024);
    }

    #[test]
    fn tabs_survive_and_invalid_utf8_is_replaced() {
        assert_eq!(
            stripped(b"\tat Foo.bar(Foo.java:3)"),
            "\tat Foo.bar(Foo.java:3)"
        );
        assert_eq!(stripped(b"caf\xc3\xa9 \xff!"), "caf\u{e9} \u{fffd}!");
    }

    /// A plain blank line reads as one; a line that only erased, moved the
    /// cursor or drew a footer at the bottom of the screen reads as none.
    #[test]
    fn a_line_that_only_redrew_reads_as_none() {
        assert_eq!(strip_line(b"\r\n").as_deref(), Some(""));
        assert_eq!(strip_line(b"").as_deref(), Some(""));
        assert_eq!(strip_line(b"abc\x1b[2K"), None);
        assert_eq!(strip_line(b"\x1b[0G\x1b[0K\r\n"), None);
        assert_eq!(
            strip_line(b"\x1b[0G\x1b[2A\x1b[s\x1b7\x1b[30;0H\x1b[0Kw Enable Watch\x1b[u\x1b8"),
            None
        );
    }
}
