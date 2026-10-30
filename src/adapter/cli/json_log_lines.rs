use std::{
    io::{self, Write},
    mem, str,
    time::{Duration, Instant},
};

use super::{followed_output::FollowedOutput, gap_marked::gap_line};
use crate::{adapter::client::LogText, domain::value::OutputCursor};

/// End of a log line.
const NEWLINE: u8 = b'\n';
/// Carriage return a terminal writes before each newline, dropped from the
/// end of a line.
const CARRIAGE_RETURN: u8 = b'\r';
/// Longest a line is held back waiting for its newline; a longer one is
/// written in pieces, each ending on a whole character.
const MAX_PENDING_LINE: usize = 64 * 1024;
/// How long a line without its newline is held back once nothing more
/// arrives: long enough to join a line written in pieces, short enough that a
/// prompt shows while it waits.
const PENDING_LINE_IDLE: Duration = Duration::from_millis(500);
/// Most bytes one UTF-8 character takes.
const MAX_CHAR_BYTES: usize = 4;
/// The bits of a byte that tell a UTF-8 continuation byte apart.
const CONTINUATION_MASK: u8 = 0b1100_0000;
/// Those bits in a continuation byte.
const CONTINUATION_BITS: u8 = 0b1000_0000;

/// A writer that turns a followed log's bytes into one JSON [`LogText`] line
/// per log line, for `muster logs -f --json`, each carrying the cursor just
/// past it. A line is held back until its newline arrives, so no object
/// carries part of a line or of a character.
pub struct JsonLogLines<'a> {
    /// Where each JSON line goes.
    out: &'a mut dyn Write,
    /// The object every line is built from; its text is replaced per line.
    template: LogText,
    /// Bytes of a line whose newline has not arrived yet.
    pending: Vec<u8>,
    /// Where the first pending byte sits in the process's output.
    next: OutputCursor,
    /// When bytes last arrived.
    last_write: Instant,
    /// Whether the line written last was a partial one the idle wait wrote
    /// out, whose newline may still arrive.
    flushed_partial: bool,
}

impl<'a> JsonLogLines<'a> {
    /// A writer of lines shaped like `template` into `out`, whose first byte
    /// sits at `start` in the process's output.
    pub fn new(out: &'a mut dyn Write, template: LogText, start: OutputCursor) -> Self {
        Self {
            out,
            template,
            pending: Vec::new(),
            next: start,
            last_write: Instant::now(),
            flushed_partial: false,
        }
    }

    /// Writes `line`, which took `consumed` bytes of output with its line
    /// ending, as one JSON object without that ending.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when the object cannot be written.
    fn emit(&mut self, line: &[u8], consumed: usize) -> io::Result<()> {
        self.next = self.next.advanced(consumed as u64);
        let line = line.strip_suffix(&[CARRIAGE_RETURN]).unwrap_or(line);
        let object = self.object(String::from_utf8_lossy(line).into_owned(), self.next, None);
        self.write_object(&object)
    }

    /// The object for `text` ending at `cursor`, shaped like the template,
    /// naming the `skipped` bytes of a gap.
    fn object(&self, text: String, cursor: OutputCursor, skipped: Option<u64>) -> LogText {
        LogText::builder()
            .project(self.template.project().clone())
            .process(self.template.process().clone())
            .log(self.template.log().clone())
            .text(text)
            .cursor(Some(cursor))
            .skipped_bytes(skipped)
            .build()
    }

    /// Writes `object` as one JSON line.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when the object cannot be written.
    fn write_object(&mut self, object: &LogText) -> io::Result<()> {
        serde_json::to_writer(&mut *self.out, object).map_err(io::Error::other)?;
        self.out.write_all(&[NEWLINE])
    }
}

impl Write for JsonLogLines<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(bytes);
        while let Some(end) = self.pending.iter().position(|byte| *byte == NEWLINE) {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            let consumed = line.len();
            let line = &line[..end];
            // The newline that ends a line already written out after the
            // idle wait carries nothing of its own.
            let ends_flushed = self.flushed_partial
                && line
                    .strip_suffix(&[CARRIAGE_RETURN])
                    .unwrap_or(line)
                    .is_empty();
            self.flushed_partial = false;
            if ends_flushed {
                self.next = self.next.advanced(consumed as u64);
            } else {
                self.emit(line, consumed)?;
            }
        }
        if self.pending.len() > MAX_PENDING_LINE {
            // Only a character cut short at the end waits for its last bytes;
            // invalid bytes never become valid, so they go out replaced.
            let whole = self.pending.len() - incomplete_tail(&self.pending);
            let piece: Vec<u8> = self.pending.drain(..whole).collect();
            self.flushed_partial = false;
            self.emit(&piece, whole)?;
        }
        self.last_write = Instant::now();
        Ok(bytes.len())
    }

    /// Writes out a held-back line once nothing has arrived for
    /// half a second, so a prompt that waits for input, such as
    /// `Password: `, is shown though no newline ends it.
    fn flush(&mut self) -> io::Result<()> {
        if !self.pending.is_empty() && self.last_write.elapsed() >= PENDING_LINE_IDLE {
            let line = mem::take(&mut self.pending);
            self.emit(&line, line.len())?;
            self.flushed_partial = true;
        }
        self.out.flush()
    }
}

impl FollowedOutput for JsonLogLines<'_> {
    /// Writes out the line the gap cut short, then one object naming the
    /// bytes lost, carrying the cursor the output resumes at.
    fn gap(&mut self, skipped: u64, resumed: OutputCursor) -> io::Result<()> {
        if !self.pending.is_empty() {
            let line = mem::take(&mut self.pending);
            self.emit(&line, line.len())?;
        }
        self.flushed_partial = false;
        self.next = resumed;
        let object = self.object(gap_line(skipped), resumed, Some(skipped));
        self.write_object(&object)
    }
}

/// How many bytes at the end of `bytes` are the start of a character whose
/// remaining bytes have not arrived, wherever any invalid byte sits before.
fn incomplete_tail(bytes: &[u8]) -> usize {
    for back in 1..MAX_CHAR_BYTES.min(bytes.len() + 1) {
        let start = bytes.len() - back;
        if bytes[start] & CONTINUATION_MASK != CONTINUATION_BITS {
            return match str::from_utf8(&bytes[start..]) {
                Err(error) if error.error_len().is_none() => back,
                _ => 0,
            };
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, thread};

    use serde_json::Value;

    use super::*;
    use crate::{
        adapter::client::ProjectIdentityView,
        domain::value::{ProcessName, ProjectName},
    };

    /// How far past the idle wait the prompt test looks.
    const IDLE_MARGIN: Duration = Duration::from_millis(100);

    /// The project every template names.
    fn shop() -> ProjectIdentityView {
        ProjectIdentityView::builder()
            .name(ProjectName::try_new("shop").unwrap())
            .root(PathBuf::from("/p"))
            .build()
    }

    /// The texts of the JSON lines `writes` produce, written one after
    /// another.
    fn texts(writes: &[&[u8]]) -> Vec<String> {
        let mut out = Vec::new();
        {
            let mut lines = JsonLogLines::new(&mut out, template("café"), OutputCursor::new(0));
            for write in writes {
                lines.write_all(write).unwrap();
            }
        }
        texts_of(out)
    }

    /// The template of process `name`'s lines.
    fn template(name: &str) -> LogText {
        LogText::builder()
            .project(shop())
            .process(ProcessName::try_new(name).unwrap())
            .log(Some(PathBuf::from("/s/api.log")))
            .text(String::new())
            .build()
    }

    /// The JSON lines in `out`.
    fn objects(out: Vec<u8>) -> Vec<Value> {
        String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    /// The texts of the JSON lines in `out`.
    fn texts_of(out: Vec<u8>) -> Vec<String> {
        objects(out)
            .iter()
            .map(|object| object["text"].as_str().unwrap().to_string())
            .collect()
    }

    /// Each line carries the cursor just past it, counted from where the
    /// writer started; a gap names the bytes lost and moves the cursor on.
    #[test]
    fn each_line_carries_its_cursor_and_a_gap_its_bytes() {
        let mut out = Vec::new();
        {
            let mut lines = JsonLogLines::new(&mut out, template("api"), OutputCursor::new(100));
            lines.write_all(b"one\r\ntwo\ncut sh").unwrap();
            lines.gap(50, OutputCursor::new(200)).unwrap();
            lines.write_all(b"after\n").unwrap();
        }

        let objects = objects(out);
        let cursors: Vec<&str> = objects
            .iter()
            .map(|object| object["cursor"].as_str().unwrap())
            .collect();
        assert_eq!(cursors, ["o:105", "o:109", "o:115", "o:200", "o:206"]);
        assert_eq!(objects[0]["process"], "api");
        assert_eq!(objects[2]["text"], "cut sh");
        assert_eq!(objects[3]["skipped_bytes"], 50);
        assert_eq!(objects[3]["text"], gap_line(50));
        assert!(objects[4].get("skipped_bytes").is_none());
    }

    /// Each log line becomes one object, however the writes split it,
    /// including a character split across two writes.
    #[test]
    fn each_log_line_is_one_object() {
        let accented = "é".as_bytes();

        assert_eq!(
            texts(&[
                b"caf",
                &accented[..1],
                &accented[1..],
                b" ready\r\nsecond\nthi"
            ]),
            vec!["café ready", "second"],
            "the unfinished third line is held back"
        );
        assert_eq!(texts(&[b"a\nb\n\n"]), vec!["a", "b", ""]);
    }

    /// A line longer than the hold-back limit is written in pieces rather
    /// than held without bound.
    #[test]
    fn an_endless_line_is_written_in_pieces() {
        let long = vec![b'x'; MAX_PENDING_LINE + 1];

        let pieces = texts(&[&long]);

        assert_eq!(pieces.len(), 1);
        assert_eq!(pieces[0].len(), MAX_PENDING_LINE + 1);
    }

    /// Invalid bytes in an over-long line go out replaced with the rest of
    /// it, not one byte at a time; only a character cut short at the end is
    /// held back for its last bytes.
    #[test]
    fn an_over_long_line_with_invalid_bytes_is_written_whole() {
        let mut invalid = vec![0xFF];
        invalid.extend(vec![b'x'; MAX_PENDING_LINE]);

        let pieces = texts(&[&invalid]);

        assert_eq!(pieces.len(), 1, "one piece, not one per invalid byte");
        assert!(pieces[0].starts_with('\u{FFFD}'));
        assert_eq!(pieces[0].chars().count(), MAX_PENDING_LINE + 1);

        let mut cut = vec![b'x'; MAX_PENDING_LINE];
        cut.push("é".as_bytes()[0]);
        let pieces = texts(&[&cut]);
        assert_eq!(pieces, vec!["x".repeat(MAX_PENDING_LINE)]);
    }

    /// An invalid byte before a character cut short at the end does not
    /// stop that character being held back for its last bytes.
    #[test]
    fn an_invalid_byte_does_not_cut_a_trailing_character() {
        let accented = "é".as_bytes();
        let mut both = vec![0xFF];
        both.extend(vec![b'x'; MAX_PENDING_LINE]);
        both.push(accented[0]);
        let mut rest = accented[1..].to_vec();
        rest.push(NEWLINE);

        let pieces = texts(&[&both, &rest]);

        assert_eq!(pieces, vec![
            format!("\u{FFFD}{}", "x".repeat(MAX_PENDING_LINE)),
            "é".to_string()
        ]);
    }

    /// The newline that ends a prompt already written after the idle wait
    /// adds no empty line of its own, while a later blank line still shows.
    #[test]
    fn a_flushed_prompt_ending_adds_no_empty_line() {
        let mut out = Vec::new();
        {
            let mut lines = JsonLogLines::new(&mut out, template("db"), OutputCursor::new(0));
            lines.write_all(b"Password: ").unwrap();
            thread::sleep(PENDING_LINE_IDLE + IDLE_MARGIN);
            lines.flush().unwrap();
            lines.write_all(b"\r\nnext\n\n").unwrap();
        }

        assert_eq!(texts_of(out), vec!["Password: ", "next", ""]);
    }

    /// A prompt with no newline is written once nothing more has arrived for
    /// the idle wait, and not before.
    #[test]
    fn a_prompt_is_written_after_the_idle_wait() {
        let mut out = Vec::new();
        {
            let mut lines = JsonLogLines::new(&mut out, template("db"), OutputCursor::new(0));
            lines.write_all(b"Password: ").unwrap();
            lines.flush().unwrap();
            assert_eq!(
                lines.pending, b"Password: ",
                "held while the line may go on"
            );
            thread::sleep(PENDING_LINE_IDLE + IDLE_MARGIN);
            lines.flush().unwrap();
            lines.flush().unwrap();
        }

        let objects = objects(out);
        assert_eq!(objects.len(), 1, "written once");
        assert_eq!(objects[0]["text"], "Password: ");
    }
}
