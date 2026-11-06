use std::{
    io::{self, Write},
    time::{Duration, Instant},
};

use super::followed_output::FollowedOutput;
use crate::{
    adapter::terminal::{
        bounded_parser::char_chunks,
        coloring::Coloring,
        escape::LINE_FEED,
        log_line::LogLine,
        log_renderer::{LogRenderer, RENDER_CHUNK_BYTES},
    },
    domain::value::OutputCursor,
};

/// How long a line without its newline is held back once nothing more
/// arrives: long enough to join a line written in pieces, short enough that a
/// prompt shows while it waits.
pub const PENDING_LINE_IDLE: Duration = Duration::from_millis(500);

/// The line `muster logs -f` prints where `skipped` bytes were rotated away
/// before they were read.
pub fn gap_line(skipped: u64) -> String {
    format!("[muster: {skipped} bytes of output were rotated away before they were read]")
}

/// A log written as the lines a log reader shows, colored as `coloring`
/// asks, with a line naming each gap: what `muster logs` prints, whole or
/// followed. Nothing that moves the cursor, erases or changes the terminal's
/// state is ever written.
///
/// The rendering starts over at each gap, and at each run start by the reset
/// the holder logs there, so one run's terminal state never reaches the
/// next. A line still being written is shown early only once nothing has
/// arrived for [`PENDING_LINE_IDLE`] and nothing has redrawn it in place, so
/// a prompt shows while a progress bar is printed once, finished; the rest
/// of a line shown early goes on after it when its newline arrives.
pub struct RenderedLog<'a> {
    /// Where the lines go.
    out: &'a mut dyn Write,
    /// The rendering, kept across writes.
    renderer: LogRenderer,
    /// Whether the lines keep their colors.
    coloring: Coloring,
    /// When bytes last arrived.
    last_write: Instant,
    /// Whether a line was written in part, its line break still to come.
    open: bool,
}

impl<'a> RenderedLog<'a> {
    /// Writes into `out`, colored as `coloring` asks, the output of a log
    /// from a line boundary on.
    pub fn new(out: &'a mut dyn Write, coloring: Coloring) -> Self {
        Self {
            out,
            renderer: LogRenderer::new(),
            coloring,
            last_write: Instant::now(),
            open: false,
        }
    }

    /// Writes out the line still being drawn as it stands, and the line
    /// break after a line written in part, as the end of the output does.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when a line cannot be written.
    pub fn finish(&mut self) -> io::Result<()> {
        self.renderer.finish();
        self.write_lines()?;
        self.close()
    }

    /// Writes every line the rendering finished.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when a line cannot be written.
    fn write_lines(&mut self) -> io::Result<()> {
        for line in self.renderer.take_lines() {
            self.write_line(&line, true)?;
        }
        Ok(())
    }

    /// Writes `line`, on a line of its own unless it goes on from the part
    /// of it written before, and its line break when `ended`.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when the line cannot be written.
    fn write_line(&mut self, line: &LogLine, ended: bool) -> io::Result<()> {
        if !line.continues() {
            self.close()?;
        }
        self.out.write_all(line.text(self.coloring).as_bytes())?;
        self.open = !ended;
        if ended {
            self.out.write_all(&[LINE_FEED])?;
        }
        Ok(())
    }

    /// Ends a line written in part.
    ///
    /// # Errors
    /// Returns an [`io::Error`] when the line break cannot be written.
    fn close(&mut self) -> io::Result<()> {
        if self.open {
            self.open = false;
            self.out.write_all(&[LINE_FEED])?;
        }
        Ok(())
    }
}

impl Write for RenderedLog<'_> {
    /// Renders `bytes` and writes every line they finish.
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        for chunk in char_chunks(bytes, RENDER_CHUNK_BYTES) {
            self.renderer.advance(chunk);
            self.write_lines()?;
        }
        self.last_write = Instant::now();
        Ok(bytes.len())
    }

    /// Writes out what a line still being drawn shows, once nothing has
    /// arrived for [`PENDING_LINE_IDLE`] and nothing redrew it in place, so a
    /// prompt that waits for input, such as `Password: `, is shown though no
    /// newline ends it, while a progress bar is not.
    fn flush(&mut self) -> io::Result<()> {
        if self.last_write.elapsed() >= PENDING_LINE_IDLE
            && self.renderer.is_settled()
            && let Some(line) = self.renderer.take_partial()
        {
            self.write_line(&line, false)?;
        }
        self.out.flush()
    }
}

impl FollowedOutput for RenderedLog<'_> {
    /// Writes out the line the gap cut short, then the line naming the bytes
    /// lost; the rendering starts over after it.
    fn gap(&mut self, skipped: u64, _resumed: OutputCursor) -> io::Result<()> {
        self.finish()?;
        self.renderer.restart();
        self.out.write_all(gap_line(skipped).as_bytes())?;
        self.out.write_all(&[LINE_FEED])
    }

    /// How many lines rendering the followed output dropped since the last
    /// call.
    fn take_faults(&mut self) -> usize {
        self.renderer.take_faults()
    }
}

#[cfg(test)]
mod tests {
    use std::{path::PathBuf, thread};

    use tempfile::TempDir;

    use super::*;
    use crate::{
        adapter::{
            cli::json_log_lines::JsonLogLines,
            client::{LogRead, LogText, ProjectIdentityView},
            holder::{
                FULL_SCREEN_LINE, LineLimit, LogQuery, gated_log::GatedLog, last_lines, read_log,
                read_log_stream,
            },
            service::OutputTap,
            terminal::render_fault,
        },
        domain::{
            project::Project,
            value::{ProcessName, ProjectName, ProjectRoot},
        },
    };

    /// How far past the idle wait the prompt tests look.
    const IDLE_MARGIN: Duration = Duration::from_millis(100);

    /// What writing each of `chunks` in turn, then ending the output, writes
    /// out, colored as `coloring` asks.
    fn rendered(chunks: &[&[u8]], coloring: Coloring) -> String {
        let mut bytes = Vec::new();
        {
            let mut out = RenderedLog::new(&mut bytes, coloring);
            for chunk in chunks {
                out.write_all(chunk).unwrap();
            }
            out.finish().unwrap();
        }
        String::from_utf8(bytes).unwrap()
    }

    /// What writing each of `chunks` writes out without color, flushing
    /// past the idle wait after each, as `logs -f` does.
    fn followed(chunks: &[&[u8]]) -> String {
        let mut bytes = Vec::new();
        {
            let mut out = RenderedLog::new(&mut bytes, Coloring::Plain);
            for chunk in chunks {
                out.write_all(chunk).unwrap();
                out.flush().unwrap();
                thread::sleep(PENDING_LINE_IDLE + IDLE_MARGIN);
                out.flush().unwrap();
            }
        }
        String::from_utf8(bytes).unwrap()
    }

    /// A gap after a line cut short starts its own line, naming the bytes,
    /// and the rendering starts over after it.
    #[test]
    fn a_gap_is_a_line_of_its_own() {
        let mut bytes = Vec::new();
        let mut out = RenderedLog::new(&mut bytes, Coloring::Plain);
        out.write_all(b"partial\x1b[?1049h").unwrap();
        out.gap(42, OutputCursor::new(100)).unwrap();
        out.write_all(b"next\n").unwrap();

        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            format!("partial\n{}\nnext\n", gap_line(42))
        );
    }

    /// A sequence split across writes is still recognized, a line is written
    /// only once its line feed arrives, and a footer redrawn between lines
    /// leaves nothing.
    #[test]
    fn lines_are_written_whole_and_redraws_leave_nothing() {
        let mut bytes = Vec::new();
        let mut out = RenderedLog::new(&mut bytes, Coloring::Colored);
        for chunk in [
            b"\x1b[3".as_slice(),
            b"2mready\x1b[0m",
            b" on :3000\r\n\x1b[0K\r\n\x1b[2A\x1b[s\x1b7\x1b[40;0Hw Enable",
            b" Watch\x1b[u\x1b8next",
        ] {
            out.write_all(chunk).unwrap();
        }

        assert_eq!(
            String::from_utf8(bytes).unwrap(),
            "\x1b[32mready\x1b[0m on :3000\n"
        );
    }

    /// A prompt with no newline is written once nothing more has arrived for
    /// the idle wait, and what follows on its line goes on after it.
    #[test]
    fn a_prompt_is_written_after_the_idle_wait() {
        assert_eq!(
            followed(&[b"Password: ", b"\r\nnext\n"]),
            "Password:\nnext\n"
        );
        assert_eq!(followed(&[b"Name? ", b"sam\r\n"]), "Name? sam\n");
    }

    /// A prompt drawn as Node's readline draws it, which returns to the
    /// first column, erases, draws and moves the cursor to the column it
    /// already stands on, is written once it stops changing, and so is one
    /// that puts the cursor back with CUB or a backspace: a cursor move
    /// redraws nothing.
    #[test]
    fn a_readline_prompt_is_written_after_the_idle_wait() {
        assert_eq!(
            followed(&[b"\x1b[1G\x1b[0JOk to proceed? (y) \x1b[20G"]),
            "Ok to proceed? (y)"
        );
        assert_eq!(followed(&[b"Name: sam\x1b[3D"]), "Name: sam");
        assert_eq!(followed(&[b"Pick: x\x08"]), "Pick: x");
    }

    /// A cargo-style progress bar that stalls past the idle wait is never
    /// written as it passes, only as `muster logs` shows it: cargo draws the
    /// bar, redraws it over itself, stalls, then erases it for the next
    /// status line.
    #[test]
    fn a_stalled_progress_bar_is_not_written() {
        let chunks: [&[u8]; 3] = [
            b"   Compiling a\n    Building [>    ] 0/3\r    Building [=>   ] 1/3: a\r",
            b"\x1b[K   Compiling b\n    Building [=>   ] 1/3\r    Building [==> ] 2/3: b\r",
            b"\x1b[K    Finished dev\n",
        ];

        let followed = followed(&chunks);

        assert_eq!(followed, rendered(&chunks, Coloring::Plain));
        assert_eq!(
            followed,
            "   Compiling a\n   Compiling b\n    Finished dev\n"
        );
    }

    /// Output that steers the terminal in the ways a log reader's rendering
    /// most easily gets wrong: the alternate screen, saved cursors jumping
    /// away, restored or not, a block climbed back over and redrawn, and a
    /// progress bar; in two runs, the first left on the alternate screen in
    /// color.
    fn steering_runs() -> (Vec<u8>, Vec<u8>) {
        let first = b"before\n\x1b[?1049h\x1b[Hfull screen\nui\n\x1b[?1049lafter\n\
            progress 40%\x1b7\x1b[30;1Hstatus bar\nnext line\n\
            \x1b7saved here\n\x1b[H\x1b[2JServer ready\n\
            a\n\x1b[s\x1b[30;1Hfooter\x1b[ub\n\
            [+] up 1/2\r\n Container Creating\r\n\x1b[2A\x1b[0G\r\n\r\n\x1b[2K Container Created\r\n\
            \x1b[31m\x1b[?1049hTUI\n"
            .to_vec();
        let second = b"   Compiling a\n    Building [=>   ] 1/3: a\r\x1b[K    Finished\n".to_vec();
        (first, second)
    }

    /// Bytes the holder's reads in the every-path test arrive in, so the
    /// screen switches are split across them.
    const HOLDER_CHUNK: usize = 7;

    /// Every path that reads a log shows the same lines for the same
    /// output, as a holder logs it: `logs`, `logs -f` fed a byte at a time,
    /// `logs --json`, `logs -f --json`, `-n`, the holder's scans and the
    /// service's tap, fed what the holder sends it.
    #[test]
    fn every_path_shows_the_same_lines() {
        let scratch = TempDir::new().unwrap();
        let path = scratch.path().join("api.log");
        let mut log = GatedLog::open(path.clone(), 1 << 20);
        let (first, second) = steering_runs();
        let mut sent = Vec::new();
        let first_start = log.start_run();
        for chunk in first.chunks(HOLDER_CHUNK) {
            sent.push(log.write(chunk));
        }
        let second_start = log.start_run();
        let second_sent = log.write(&second);
        let (_, stream) = read_log_stream(&path).unwrap();
        let expected = [
            "before",
            FULL_SCREEN_LINE,
            "after",
            "progress 40%",
            "next line",
            "saved here",
            "Server ready",
            "a",
            "b",
            "[+] up 1/2",
            " Container Creating",
            " Container Created",
            FULL_SCREEN_LINE,
            "   Compiling a",
            "    Finished",
        ];

        let mut whole = Vec::new();
        {
            let mut out = RenderedLog::new(&mut whole, Coloring::Plain);
            out.write_all(&stream).unwrap();
            out.finish().unwrap();
        }
        let mut followed = Vec::new();
        {
            let mut out = RenderedLog::new(&mut followed, Coloring::Plain);
            for byte in &stream {
                out.write_all(&[*byte]).unwrap();
            }
            out.finish().unwrap();
        }
        let mut objects = Vec::new();
        {
            let template = LogText::builder()
                .project(
                    ProjectIdentityView::builder()
                        .name(ProjectName::try_new("shop").unwrap())
                        .root(PathBuf::from("/p"))
                        .build(),
                )
                .process(ProcessName::try_new("api").unwrap())
                .log(Some(path.clone()))
                .text(String::new())
                .build();
            let mut out = JsonLogLines::new(&mut objects, template, OutputCursor::new(0));
            for byte in &stream {
                out.write_all(&[*byte]).unwrap();
            }
        }
        let project = Project::builder()
            .root(ProjectRoot::try_new(PathBuf::from("/p")).unwrap())
            .name(ProjectName::try_new("shop").unwrap())
            .build();
        let read = LogRead::builder()
            .name(ProcessName::try_new("api").unwrap())
            .log(Some(path.clone()))
            .end(Some(log.end()))
            .bytes(stream.clone())
            .build();
        let newest = read_log(
            &path,
            &LogQuery::builder()
                .lines(LineLimit::try_new(expected.len()).unwrap())
                .build(),
        )
        .unwrap();
        let scanned = last_lines(
            &path,
            OutputCursor::new(0),
            log.end(),
            expected.len(),
            stream.len() as u64,
        )
        .unwrap();
        let mut tap = OutputTap::new(OutputCursor::new(0));
        let mut tapped = Vec::new();
        tap.begin_run_at(first_start);
        for (at, bytes) in &sent {
            tapped.extend(tap.feed(Some(*at), bytes));
        }
        tap.begin_run_at(second_start);
        tapped.extend(tap.feed(Some(second_sent.0), &second_sent.1));
        tapped.extend(tap.unfinished());
        let tapped: Vec<String> = tapped.iter().map(|line| line.text().clone()).collect();

        let lines = |bytes: &[u8]| -> Vec<String> {
            String::from_utf8(bytes.to_vec())
                .unwrap()
                .lines()
                .map(str::to_string)
                .collect()
        };
        let json_texts: Vec<String> = String::from_utf8(objects)
            .unwrap()
            .lines()
            .map(|line| {
                let object: serde_json::Value = serde_json::from_str(line).unwrap();
                object["text"].as_str().unwrap().to_string()
            })
            .collect();
        assert_eq!(lines(&whole), expected, "logs");
        assert_eq!(lines(&followed), expected, "logs -f");
        assert_eq!(json_texts, expected, "logs -f --json");
        assert_eq!(
            lines(read.text_in(&project).text().as_bytes()),
            expected,
            "logs --json"
        );
        assert_eq!(*newest.lines(), expected, "logs -n");
        assert_eq!(*scanned.lines(), expected, "wait and readiness scans");
        assert_eq!(tapped, expected, "the service's tap");
    }

    /// A rendering fault drops only the line it hit, and the writer counts
    /// it for the warning `logs` prints.
    #[test]
    fn a_rendering_fault_drops_one_line_and_is_counted() {
        let mut bytes = Vec::new();
        let dropped = {
            let mut out = RenderedLog::new(&mut bytes, Coloring::Plain);
            render_fault::arm();
            out.write_all(b"lost\nkept\n").unwrap();
            out.finish().unwrap();
            out.take_faults()
        };

        assert_eq!(String::from_utf8(bytes).unwrap(), "kept\n");
        assert_eq!(dropped, 1);
    }

    /// A C1 control split across writes, which the parser hands over as a
    /// printed character, never reaches the output: not a CSI clearing the
    /// screen, nor an OSC 52 clipboard write ended by ST.
    #[test]
    fn a_split_c1_control_is_dropped() {
        let text = rendered(
            &[
                b"x\xC2",
                b"\x9B2J\n",
                b"y\xC2",
                b"\x9D52;c;aGk=\xC2",
                b"\x9C\n",
            ],
            Coloring::Colored,
        );

        assert_eq!(text, "x2J\ny52;c;aGk=\n");
    }
}
