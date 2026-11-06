use std::{
    io::{self, Write},
    time::Instant,
};

use super::{
    followed_output::FollowedOutput,
    rendered_log::{PENDING_LINE_IDLE, gap_line},
};
use crate::{
    adapter::{
        client::LogText,
        terminal::{escape::LINE_FEED, log_line::LogLine, placed_renderer::PlacedRenderer},
    },
    domain::value::OutputCursor,
};

/// A writer that turns a followed log's bytes into one JSON [`LogText`] line
/// per log line, for `muster logs -f --json`, each carrying the cursor just
/// past it and the line's text as `muster logs` renders it, without color;
/// a line that only redrew the screen moves the cursor on and writes nothing.
/// A line is written once its newline arrives, or early, as `logs -f` shows
/// it, once nothing arrived for `PENDING_LINE_IDLE` and nothing redrew it
/// in place; the rest of it then follows in an object of its own.
pub struct JsonLogLines<'a> {
    /// Where each JSON line goes.
    out: &'a mut dyn Write,
    /// The object every line is built from; its text is replaced per line.
    template: LogText,
    /// The rendering, kept across lines, placed in the process's output.
    renderer: PlacedRenderer,
    /// When bytes last arrived.
    last_write: Instant,
}

impl<'a> JsonLogLines<'a> {
    /// A writer of lines shaped like `template` into `out`, whose first byte
    /// sits at `start` in the process's output, on a line boundary.
    pub fn new(out: &'a mut dyn Write, template: LogText, start: OutputCursor) -> Self {
        Self {
            out,
            template,
            renderer: PlacedRenderer::new(start),
            last_write: Instant::now(),
        }
    }

    /// Writes one object for each line the rendering finished.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when an object cannot be written.
    fn write_lines(&mut self) -> io::Result<()> {
        for line in self.renderer.take_lines() {
            self.write_line(&line)?;
        }
        Ok(())
    }

    /// Writes `line` as one object carrying the cursor where the rendering
    /// stands.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when the object cannot be written.
    fn write_line(&mut self, line: &LogLine) -> io::Result<()> {
        let object = self.object(line.plain(), self.renderer.next(), None);
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
        self.out.write_all(&[LINE_FEED])
    }
}

impl Write for JsonLogLines<'_> {
    /// Renders `bytes` a line at a time, so each object carries the cursor
    /// just past the line it ends.
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        for piece in bytes.split_inclusive(|byte| *byte == LINE_FEED) {
            self.renderer.advance(piece);
            self.write_lines()?;
        }
        self.last_write = Instant::now();
        Ok(bytes.len())
    }

    /// Writes out what a line still being drawn shows once nothing has
    /// arrived for `PENDING_LINE_IDLE` and nothing redrew it in place, so
    /// a prompt that waits for input, such as `Password: `, is shown though
    /// no newline ends it, while a progress bar is not.
    fn flush(&mut self) -> io::Result<()> {
        if self.last_write.elapsed() >= PENDING_LINE_IDLE
            && self.renderer.is_settled()
            && let Some(line) = self.renderer.take_partial()
        {
            self.write_line(&line)?;
        }
        self.out.flush()
    }
}

impl FollowedOutput for JsonLogLines<'_> {
    /// Writes out the line the gap cut short, then one object naming the
    /// bytes lost, carrying the cursor the output resumes at; the rendering
    /// starts over there.
    fn gap(&mut self, skipped: u64, resumed: OutputCursor) -> io::Result<()> {
        self.renderer.finish();
        self.write_lines()?;
        self.renderer.skip_to(resumed);
        let object = self.object(gap_line(skipped), resumed, Some(skipped));
        self.write_object(&object)
    }

    /// How many lines rendering the followed output dropped since the last
    /// call.
    fn take_faults(&mut self) -> usize {
        self.renderer.take_faults()
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, thread, time::Duration};

    use serde_json::Value;

    use super::*;
    use crate::{
        adapter::{client::ProjectIdentityView, terminal::line_drawing::MAX_LINE_CELLS},
        domain::value::{ProcessName, ProjectName},
    };

    /// How far past the idle wait the prompt tests look.
    const IDLE_MARGIN: Duration = Duration::from_millis(100);

    /// The project every template names.
    fn shop() -> ProjectIdentityView {
        ProjectIdentityView::builder()
            .name(ProjectName::try_new("shop").unwrap())
            .root(PathBuf::from("/p"))
            .build()
    }

    /// The template of process `name`'s lines, whose log records no run
    /// starts.
    fn template(name: &str) -> LogText {
        LogText::builder()
            .project(shop())
            .process(ProcessName::try_new(name).unwrap())
            .log(Some(PathBuf::from("/nonexistent/muster/api.log")))
            .text(String::new())
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

    /// The texts of the JSON lines `writes` produce, flushing past the idle
    /// wait after each, as `logs -f --json` does.
    fn followed(writes: &[&[u8]]) -> Vec<String> {
        let mut out = Vec::new();
        {
            let mut lines = JsonLogLines::new(&mut out, template("db"), OutputCursor::new(0));
            for write in writes {
                lines.write_all(write).unwrap();
                lines.flush().unwrap();
                thread::sleep(PENDING_LINE_IDLE + IDLE_MARGIN);
                lines.flush().unwrap();
            }
        }
        texts_of(out)
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

    /// A line longer than a line holds is written in pieces rather than held
    /// without bound.
    #[test]
    fn an_endless_line_is_written_in_pieces() {
        let long = vec![b'x'; MAX_LINE_CELLS + 1];

        assert_eq!(texts(&[&long]), vec!["x".repeat(MAX_LINE_CELLS)]);
    }

    /// A prompt is held while the line may go on, then written once nothing
    /// more has arrived for the idle wait; its newline adds no empty line of
    /// its own, while a later blank line still shows.
    #[test]
    fn a_prompt_is_written_after_the_idle_wait() {
        let mut out = Vec::new();
        {
            let mut lines = JsonLogLines::new(&mut out, template("db"), OutputCursor::new(0));
            lines.write_all(b"Password: ").unwrap();
            lines.flush().unwrap();
        }
        assert!(out.is_empty(), "held while the line may go on");
        assert_eq!(followed(&[b"Password: ", b"\r\nnext\n\n"]), vec![
            "Password:",
            "next",
            ""
        ]);
    }

    /// A prompt drawn as Node's readline draws it, which returns to the
    /// first column, erases, draws and moves the cursor to the column it
    /// already stands on, is written once it stops changing, and so is one
    /// that puts the cursor back with CUB or a backspace.
    #[test]
    fn a_readline_prompt_is_written_after_the_idle_wait() {
        assert_eq!(
            followed(&[b"\x1b[1G\x1b[0JOk to proceed? (y) \x1b[20G"]),
            vec!["Ok to proceed? (y)"]
        );
        assert_eq!(followed(&[b"Name: sam\x1b[3D"]), vec!["Name: sam"]);
        assert_eq!(followed(&[b"Pick: x\x08"]), vec!["Pick: x"]);
    }

    /// A cargo-style progress bar that redraws over itself and stalls past
    /// the idle wait is never written as it passes: only the lines `muster
    /// logs` shows are.
    #[test]
    fn a_stalled_progress_bar_is_not_written() {
        assert_eq!(
            followed(&[
                b"   Compiling a\n    Building [>    ] 0/3\r    Building [=>   ] 1/3: a\r",
                b"\x1b[K   Compiling b\n    Building [=>   ] 1/3\r    Building [==> ] 2/3: b\r",
                b"\x1b[K    Finished dev\n",
            ]),
            vec!["   Compiling a", "   Compiling b", "    Finished dev"]
        );
    }

    /// Each line's text is rendered as `muster logs` shows it, without
    /// color; a footer redrawn between lines and the room made for it write
    /// no object, yet the next line's cursor still counts their bytes.
    #[test]
    fn a_line_carries_its_rendered_text_and_redraws_carry_none() {
        let footer = b"\x1b[0K\r\n\x1b[2A\x1b[s\x1b7\x1b[40;0Hw Enable Watch\x1b[u\x1b8";
        let mut stream = b"\x1b[36mweb-1  | \x1b[0mhello\r\n".to_vec();
        stream.extend_from_slice(footer);
        stream.extend_from_slice(b"10%\r\x1b[2K100% done\r\n");
        let mut out = Vec::new();
        {
            let mut lines = JsonLogLines::new(&mut out, template("web"), OutputCursor::new(0));
            lines.write_all(&stream).unwrap();
        }

        let objects = objects(out);
        let texts: Vec<&str> = objects
            .iter()
            .map(|object| object["text"].as_str().unwrap())
            .collect();
        assert_eq!(texts, ["web-1  | hello", "100% done"]);
        assert_eq!(
            objects[1]["cursor"].as_str().unwrap(),
            format!("o:{}", stream.len())
        );
    }

    /// A C1 control split across writes, or across an idle flush, never
    /// reaches an object's text.
    #[test]
    fn a_split_c1_control_never_reaches_the_text() {
        assert_eq!(texts(&[b"x\xC2", b"\x9B2J\n"]), vec!["x2J"]);
        assert_eq!(followed(&[b"y\xC2", b"\x9D52;c\xC2", b"\x9C\n"]), vec![
            "y", "52;c"
        ]);
    }
}
