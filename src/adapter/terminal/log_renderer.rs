//! A process's raw output rendered as the lines a log reader shows: the one
//! rendering behind `muster logs`, `logs -f`, their JSON, and every line the
//! holder reads, matches and returns to an agent.

use super::{
    bounded_parser::BoundedParser, colouring::Colouring, line_drawing::LineDrawing,
    log_line::LogLine,
};

/// Ends each rendered line.
const LINE_BREAK: char = '\n';
/// Most bytes to render before taking the lines they finish, so a whole log
/// never holds every line's cells at once.
pub const RENDER_CHUNK_BYTES: usize = 64 * 1024;

/// Renders raw output, fed in pieces of any size, into finished
/// [`LogLine`]s: each line's carriage returns, erases and column moves
/// applied, and blank lines that only redrew the screen, footers drawn at a
/// jump from a saved cursor, and the alternate screen left out.
#[derive(Default)]
pub struct LogRenderer {
    /// The parser, kept across pieces.
    parser: BoundedParser,
    /// The line being drawn and the lines finished.
    drawing: LineDrawing,
}

impl LogRenderer {
    /// A renderer at the start of a stream.
    pub fn new() -> Self {
        Self::default()
    }

    /// Renders `bytes`, the next piece of the stream.
    pub fn advance(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.drawing, bytes);
    }

    /// The lines finished since the last call, oldest first.
    pub fn take_lines(&mut self) -> Vec<LogLine> {
        self.drawing.take_lines()
    }

    /// The line still being drawn, unless it is blank; what follows on it
    /// starts a line of its own.
    pub fn take_partial(&mut self) -> Option<LogLine> {
        self.drawing.take_partial()
    }

    /// Every line of `bytes` rendered as `colouring` asks, each ending in a
    /// line break, the unfinished last one too.
    pub fn text_of(bytes: &[u8], colouring: Colouring) -> String {
        let mut renderer = Self::new();
        let mut text = String::new();
        let mut append = |lines: Vec<LogLine>| {
            for line in lines {
                text.push_str(&line.text(colouring));
                text.push(LINE_BREAK);
            }
        };
        for chunk in bytes.chunks(RENDER_CHUNK_BYTES) {
            renderer.advance(chunk);
            append(renderer.take_lines());
        }
        append(renderer.take_partial().into_iter().collect());
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::terminal::{line_drawing::MAX_LINE_CELLS, pen::MAX_PEN_BYTES};

    /// What `bytes` render to in colour.
    fn coloured(bytes: &[u8]) -> String {
        LogRenderer::text_of(bytes, Colouring::Coloured)
    }

    /// What `bytes` render to without colour.
    fn plain(bytes: &[u8]) -> String {
        LogRenderer::text_of(bytes, Colouring::Plain)
    }

    /// The bytes `docker compose up` writes on a terminal around two log
    /// lines, as captured from it: each line clears the footer below it,
    /// makes room for the footer again, climbs back and redraws the
    /// `w Enable Watch   d Detach` footer at the bottom of the screen
    /// between a saved and a restored cursor.
    fn compose_footer_stream() -> Vec<u8> {
        let clear_below = format!(
            "\x1b[0G\x1b[s\x1b7{}\x1b[u\x1b8",
            "\x1b[1B\x1b[0K".repeat(30)
        );
        let footer = "\x1b[0G\x1b[2A\x1b[0G\x1b[s\x1b7\x1b[30;0H\x1b[0K\
            \x1b[1;38;2;0;0;0;48;2;255;255;255mw\x1b[0m\x1b[2m Enable Watch\x1b[0m   \
            \x1b[1;38;2;0;0;0;48;2;255;255;255md\x1b[0m\x1b[2m Detach\x1b[0m\x1b[0G\x1b[u\x1b8";
        let make_room = "\x1b[0K\r\n\x1b[0G\x1b[0K\r\n";
        let mut stream = String::from("\x1b[?25hAttaching to web-1\r\n");
        for line in ["hello 1", "hello 2"] {
            stream.push_str(&clear_below);
            stream.push_str(&format!("\x1b[36mweb-1  | \x1b[0m{line}\r\n"));
            stream.push_str(&clear_below);
            stream.push_str(make_room);
            stream.push_str(footer);
        }
        stream.into_bytes()
    }

    /// Docker compose's footer and the room it makes for it leave only the
    /// real lines, each coloured as drawn and reset at its end.
    #[test]
    fn a_compose_footer_leaves_only_the_log_lines() {
        let stream = compose_footer_stream();

        assert_eq!(
            plain(&stream),
            "Attaching to web-1\nweb-1  | hello 1\nweb-1  | hello 2\n"
        );
        assert_eq!(
            coloured(&stream),
            "Attaching to web-1\n\x1b[36mweb-1  | \x1b[0mhello 1\n\x1b[36mweb-1  | \x1b[0mhello 2\n"
        );
    }

    /// The progress compose draws while it starts climbs back over the
    /// lines it redraws, passing unchanged ones with a bare line break; those
    /// add no blank lines, while a blank line printed plainly stays.
    #[test]
    fn lines_a_redraw_passes_over_add_no_blank_lines() {
        let stream = b"[+] up 1/2\r\n Container Creating\r\n\x1b[?25l\x1b[2A\x1b[0G\r\n\r\n\
            \x1b[2K Container Created\r\nnext\r\n\r\nafter a blank\r\n";

        assert_eq!(
            plain(stream),
            "[+] up 1/2\n Container Creating\n Container Created\nnext\n\nafter a blank\n"
        );
    }

    /// A progress bar redrawn with carriage returns and erases reads as its
    /// last state, and clearing it before its line feed leaves no line.
    #[test]
    fn a_progress_bar_collapses_to_its_last_state() {
        assert_eq!(plain(b"10%\r50%\r100% done\n"), "100% done\n");
        assert_eq!(plain(b"downloading 1/3\r\x1b[2K\x1b[1Gok\r\n"), "ok\n");
        assert_eq!(plain(b"working\r\x1b[K\nnext\n"), "next\n");
        assert_eq!(
            coloured(b"\x1b[32m10%\x1b[0m\r\x1b[2K\x1b[32m100%\x1b[0m done\n"),
            "\x1b[32m100%\x1b[0m done\n"
        );
    }

    /// A title, a clipboard write, a query, a DCS, a scroll region, a mode
    /// change and an alternate screen leave nothing; the text beside them
    /// stays, and a line holding nothing else is dropped.
    #[test]
    fn sequences_that_steer_the_terminal_leave_nothing() {
        assert_eq!(
            coloured(
                b"\x1b]0;pwned\x07title\x1b]52;c;aGk=\x1b\\ \x1b[6n\x1b[18t\x1bP1$qm\x1b\\ok\n"
            ),
            "title ok\n"
        );
        assert_eq!(
            plain(b"\x1b[1;20r\x1b[?1h\x1b=\r\nregion set\n\x1b[r\n"),
            "region set\n"
        );
        assert_eq!(
            plain(b"before\n\x1b[?1049h\x1b[H\x1b[2Jfull screen\nui\n\x1b[?1049lafter\n"),
            "before\nafter\n"
        );
        assert_eq!(
            plain(b"a\n\x1b[s\x1b[30;1Hfooter\x1b[ub\n\x1b7\x1b[30dfooter\x1b8c\n"),
            "a\nb\nc\n"
        );
        assert_eq!(
            plain(b"\x1b7\x1b[30;1Hstatus\nnot lost\n\x1b[1;3Hab\n"),
            "not lost\n  ab\n",
            "a line feed ends a jump that is never restored, and its save"
        );
    }

    /// Colours outlive a line break as they do on a terminal, yet every line
    /// ends reset, and a bold red run reads back as one sequence.
    #[test]
    fn colours_carry_across_lines_but_never_leak() {
        assert_eq!(
            coloured(b"\x1b[1m\x1b[31mred\nstill red\x1b[0m plain\n"),
            "\x1b[1;31mred\x1b[0m\n\x1b[1;31mstill red\x1b[0m plain\n"
        );
        assert_eq!(
            coloured(b"\x1b[38;5;0mblack\x1b[0m\n"),
            "\x1b[38;5;0mblack\x1b[0m\n"
        );
        assert_eq!(coloured(b"\x1b[31m   \n"), "\n");
    }

    /// A pen that is never reset keeps only its newest attributes.
    #[test]
    fn a_pen_never_reset_stays_bounded() {
        let mut stream = b"\x1b[31m".repeat(MAX_PEN_BYTES);
        stream.extend_from_slice(b"x\n");

        let text = coloured(&stream);

        assert!(text.len() < MAX_PEN_BYTES * 2, "{text}");
        assert!(text.contains("x\x1b[0m"), "{text}");
    }

    /// An unfinished last line is shown, a line feed alone is a blank line,
    /// and a line far longer than any screen is cut, not held without bound.
    #[test]
    fn unfinished_and_endless_lines() {
        assert_eq!(plain(b"one\ntwo"), "one\ntwo\n");
        assert_eq!(plain(b"\n"), "\n");
        let endless = vec![b'x'; MAX_LINE_CELLS + 1];
        let text = plain(&endless);
        assert_eq!(text.lines().map(str::len).collect::<Vec<_>>(), vec![
            MAX_LINE_CELLS,
            1
        ]);
    }

    /// A line shown in part goes on as a line of its own, and its line feed
    /// alone adds no blank line.
    #[test]
    fn a_line_shown_in_part_goes_on_without_a_blank_line() {
        let mut renderer = LogRenderer::new();
        renderer.advance(b"Password: ");
        assert_eq!(
            renderer.take_partial().map(|line| line.plain()),
            Some("Password:".to_string())
        );
        renderer.advance(b"\r\nnext\n\n");

        let lines: Vec<String> = renderer.take_lines().iter().map(LogLine::plain).collect();

        assert_eq!(lines, vec!["next", ""]);
        assert_eq!(renderer.take_partial(), None);
    }
}
