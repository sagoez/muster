use std::{
    io::{self, Write},
    time::{Duration, Instant},
};

use super::followed_output::FollowedOutput;
use crate::{
    adapter::terminal::{
        colouring::Colouring,
        log_line::LogLine,
        log_renderer::{LogRenderer, RENDER_CHUNK_BYTES},
    },
    domain::value::OutputCursor,
};

/// Ends a line.
const LINE_FEED: u8 = b'\n';
/// How long a line without its newline is held back once nothing more
/// arrives: long enough to join a line written in pieces, short enough that a
/// prompt shows while it waits.
pub const PENDING_LINE_IDLE: Duration = Duration::from_millis(500);

/// The line `muster logs -f` prints where `skipped` bytes were rotated away
/// before they were read.
pub fn gap_line(skipped: u64) -> String {
    format!("[muster: {skipped} bytes of output were rotated away before they were read]")
}

/// A followed log written as the lines a log reader shows, coloured as
/// `colouring` asks, with a line naming each gap. Nothing that moves the
/// cursor, erases or changes the terminal's state is ever written.
pub struct RenderedLog<'a> {
    /// Where the lines go.
    out: &'a mut dyn Write,
    /// The rendering, kept across writes.
    renderer: LogRenderer,
    /// Whether the lines keep their colours.
    colouring: Colouring,
    /// When bytes last arrived.
    last_write: Instant,
}

impl<'a> RenderedLog<'a> {
    /// Writes into `out`, coloured as `colouring` asks.
    pub fn new(out: &'a mut dyn Write, colouring: Colouring) -> Self {
        Self {
            out,
            renderer: LogRenderer::new(),
            colouring,
            last_write: Instant::now(),
        }
    }

    /// Writes `line` and its line feed.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when the line cannot be written.
    fn write_line(&mut self, line: &LogLine) -> io::Result<()> {
        self.out.write_all(line.text(self.colouring).as_bytes())?;
        self.out.write_all(&[LINE_FEED])
    }

    /// Writes out the line still being drawn, if anything is on it.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when the line cannot be written.
    fn write_partial(&mut self) -> io::Result<()> {
        match self.renderer.take_partial() {
            Some(line) => self.write_line(&line),
            None => Ok(()),
        }
    }
}

impl Write for RenderedLog<'_> {
    /// Renders `bytes` and writes every line they finish.
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        for chunk in bytes.chunks(RENDER_CHUNK_BYTES) {
            self.renderer.advance(chunk);
            for line in self.renderer.take_lines() {
                self.write_line(&line)?;
            }
        }
        self.last_write = Instant::now();
        Ok(bytes.len())
    }

    /// Writes out a held-back line once nothing has arrived for
    /// [`PENDING_LINE_IDLE`], so a prompt that waits for input, such as
    /// `Password: `, is shown though no newline ends it.
    fn flush(&mut self) -> io::Result<()> {
        if self.last_write.elapsed() >= PENDING_LINE_IDLE {
            self.write_partial()?;
        }
        self.out.flush()
    }
}

impl FollowedOutput for RenderedLog<'_> {
    /// Writes out the line the gap cut short, then the line naming the bytes
    /// lost.
    fn gap(&mut self, skipped: u64, _resumed: OutputCursor) -> io::Result<()> {
        self.write_partial()?;
        self.out.write_all(gap_line(skipped).as_bytes())?;
        self.out.write_all(&[LINE_FEED])
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::*;

    /// How far past the idle wait the prompt test looks.
    const IDLE_MARGIN: Duration = Duration::from_millis(100);

    /// What writing each of `chunks` in turn writes out, coloured as
    /// `colouring` asks.
    fn rendered(chunks: &[&[u8]], colouring: Colouring) -> String {
        let mut bytes = Vec::new();
        let mut out = RenderedLog::new(&mut bytes, colouring);
        for chunk in chunks {
            out.write_all(chunk).unwrap();
        }
        String::from_utf8(bytes).unwrap()
    }

    /// A gap after a line cut short starts its own line, naming the bytes.
    #[test]
    fn a_gap_is_a_line_of_its_own() {
        let mut bytes = Vec::new();
        let mut out = RenderedLog::new(&mut bytes, Colouring::Plain);
        out.write_all(b"partial").unwrap();
        out.gap(42, OutputCursor::new(100)).unwrap();
        out.write_all(b"next\n").unwrap();

        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            format!("partial\n{}\nnext\n", gap_line(42))
        );
    }

    /// A sequence split across writes is still recognised, a line is written
    /// only once its line feed arrives, and a footer redrawn between lines
    /// leaves nothing.
    #[test]
    fn lines_are_written_whole_and_redraws_leave_nothing() {
        assert_eq!(
            rendered(
                &[
                    b"\x1b[3",
                    b"2mready\x1b[0m",
                    b" on :3000\r\n\x1b[0K\r\n\x1b[2A\x1b[s\x1b7\x1b[40;0Hw Enable",
                    b" Watch\x1b[u\x1b8next"
                ],
                Colouring::Coloured
            ),
            "\x1b[32mready\x1b[0m on :3000\n"
        );
    }

    /// A prompt with no newline is written once nothing more has arrived for
    /// the idle wait, and its newline then adds no blank line.
    #[test]
    fn a_prompt_is_written_after_the_idle_wait() {
        let mut bytes = Vec::new();
        {
            let mut out = RenderedLog::new(&mut bytes, Colouring::Plain);
            out.write_all(b"Password: ").unwrap();
            out.flush().unwrap();
            thread::sleep(PENDING_LINE_IDLE + IDLE_MARGIN);
            out.flush().unwrap();
            out.write_all(b"\r\nnext\n").unwrap();
        }

        assert_eq!(String::from_utf8(bytes).unwrap(), "Password:\nnext\n");
    }
}
