use std::{
    io::{self, Write},
    mem, str,
    time::{Duration, Instant},
};

use crate::adapter::client::LogText;

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
/// per log line, for `muster logs -f --json`. A line is held back until its
/// newline arrives, so no object carries part of a line or of a character.
pub struct JsonLogLines<'a> {
    /// Where each JSON line goes.
    out: &'a mut dyn Write,
    /// The object every line is built from; its text is replaced per line.
    template: LogText,
    /// Bytes of a line whose newline has not arrived yet.
    pending: Vec<u8>,
    /// When bytes last arrived.
    last_write: Instant,
    /// Whether the line written last was a partial one the idle wait wrote
    /// out, whose newline may still arrive.
    flushed_partial: bool,
}

impl<'a> JsonLogLines<'a> {
    /// A writer of lines shaped like `template` into `out`.
    pub fn new(out: &'a mut dyn Write, template: LogText) -> Self {
        Self {
            out,
            template,
            pending: Vec::new(),
            last_write: Instant::now(),
            flushed_partial: false,
        }
    }

    /// Writes `line` as one JSON object, without its line ending.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when the object cannot be written.
    fn emit(&mut self, line: &[u8]) -> io::Result<()> {
        let line = line.strip_suffix(&[CARRIAGE_RETURN]).unwrap_or(line);
        let object = LogText::builder()
            .project(self.template.project().clone())
            .name(self.template.name().clone())
            .log(self.template.log().clone())
            .text(String::from_utf8_lossy(line).into_owned())
            .build();
        serde_json::to_writer(&mut *self.out, &object).map_err(io::Error::other)?;
        self.out.write_all(&[NEWLINE])
    }
}

impl Write for JsonLogLines<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(bytes);
        while let Some(end) = self.pending.iter().position(|byte| *byte == NEWLINE) {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            let line = &line[..end];
            // The newline that ends a line already written out after the
            // idle wait carries nothing of its own.
            let ends_flushed = self.flushed_partial
                && line
                    .strip_suffix(&[CARRIAGE_RETURN])
                    .unwrap_or(line)
                    .is_empty();
            self.flushed_partial = false;
            if !ends_flushed {
                self.emit(line)?;
            }
        }
        if self.pending.len() > MAX_PENDING_LINE {
            // Only a character cut short at the end waits for its last bytes;
            // invalid bytes never become valid, so they go out replaced.
            let whole = self.pending.len() - incomplete_tail(&self.pending);
            let piece: Vec<u8> = self.pending.drain(..whole).collect();
            self.flushed_partial = false;
            self.emit(&piece)?;
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
            self.emit(&line)?;
            self.flushed_partial = true;
        }
        self.out.flush()
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
        let template = LogText::builder()
            .project(shop())
            .name(ProcessName::try_new("café").unwrap())
            .log(Some(PathBuf::from("/s/api.log")))
            .text(String::new())
            .build();
        let mut out = Vec::new();
        {
            let mut lines = JsonLogLines::new(&mut out, template);
            for write in writes {
                lines.write_all(write).unwrap();
            }
        }
        texts_of(out)
    }

    /// The texts of the JSON lines in `out`.
    fn texts_of(out: Vec<u8>) -> Vec<String> {
        String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|line| {
                serde_json::from_str::<Value>(line).unwrap()["text"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect()
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
        let template = LogText::builder()
            .project(shop())
            .name(ProcessName::try_new("db").unwrap())
            .log(None)
            .text(String::new())
            .build();
        let mut out = Vec::new();
        {
            let mut lines = JsonLogLines::new(&mut out, template);
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
        let template = LogText::builder()
            .project(shop())
            .name(ProcessName::try_new("db").unwrap())
            .log(None)
            .text(String::new())
            .build();
        let mut out = Vec::new();
        {
            let mut lines = JsonLogLines::new(&mut out, template);
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

        let objects: Vec<Value> = String::from_utf8(out)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(objects.len(), 1, "written once");
        assert_eq!(objects[0]["text"], "Password: ");
    }
}
