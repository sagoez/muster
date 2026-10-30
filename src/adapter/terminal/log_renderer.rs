//! A process's raw output rendered as the lines a log reader shows: the one
//! rendering behind `muster logs`, `logs -f`, their JSON, the service's
//! readiness and waits, and every line the holder reads, matches and returns
//! to an agent.

use super::{bounded_parser::BoundedParser, line_drawing::LineDrawing, log_line::LogLine};

/// Most bytes to render before taking the lines they finish, so a whole log
/// never holds every line's cells at once.
pub const RENDER_CHUNK_BYTES: usize = 64 * 1024;

/// Renders raw output, fed in pieces of any size, into finished
/// [`LogLine`]s: each line's carriage returns, erases and column moves
/// applied, and blank lines that only redrew the screen, footers drawn at a
/// jump from a saved cursor, and the alternate screen left out. Each byte is
/// rendered once, so feeding a line in many pieces costs what feeding it
/// whole does.
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

    /// Whether the line still being drawn was never redrawn in place, so it
    /// may be shown before its line feed.
    pub fn is_settled(&self) -> bool {
        self.drawing.is_settled()
    }

    /// What the line still being drawn shows past what was taken of it
    /// before, unless that is blank; the line goes on, and its line feed
    /// yields only what follows, or all of it when the part taken changed.
    pub fn take_partial(&mut self) -> Option<LogLine> {
        self.drawing.take_partial()
    }

    /// The whole line still being drawn, unless it is blank, leaving it be.
    pub fn peek_partial(&self) -> Option<LogLine> {
        self.drawing.peek()
    }

    /// Ends the line still being drawn as the end of the output does; it is
    /// among the next lines taken unless nothing is left of it to show.
    pub fn finish(&mut self) {
        self.drawing.finish();
    }

    /// Ends the line still being drawn, then starts over as a fresh
    /// terminal: no sequence in progress, pen, saved cursor or alternate
    /// screen outlives this point.
    pub fn restart(&mut self) {
        self.drawing.reset();
        self.parser = BoundedParser::new();
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;
    use crate::adapter::terminal::{
        coloring::Coloring,
        line_drawing::{MAX_COLUMN, MAX_LINE_CELLS},
    };

    /// Erases to the cursor the long-line test feeds.
    const ERASES: usize = 200_000;
    /// How long those erases may take; filling the line for each would take
    /// minutes.
    const ERASE_BOUND: Duration = Duration::from_secs(2);

    /// Every line of `bytes` rendered as `coloring` asks, each ending in a
    /// line break, the unfinished last one too.
    fn text_of(bytes: &[u8], coloring: Coloring) -> String {
        let mut renderer = LogRenderer::new();
        renderer.advance(bytes);
        renderer.finish();
        renderer
            .take_lines()
            .iter()
            .map(|line| format!("{}\n", line.text(coloring)))
            .collect()
    }

    /// What `bytes` render to in color.
    fn colored(bytes: &[u8]) -> String {
        text_of(bytes, Coloring::Colored)
    }

    /// What `bytes` render to without color.
    fn plain(bytes: &[u8]) -> String {
        text_of(bytes, Coloring::Plain)
    }

    /// The text and continuation of each line `renderer` finished.
    fn taken(renderer: &mut LogRenderer) -> Vec<(String, bool)> {
        renderer
            .take_lines()
            .iter()
            .map(|line| (line.plain(), line.continues()))
            .collect()
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
    /// real lines, each colored as drawn and reset at its end.
    #[test]
    fn a_compose_footer_leaves_only_the_log_lines() {
        let stream = compose_footer_stream();

        assert_eq!(
            plain(&stream),
            "Attaching to web-1\nweb-1  | hello 1\nweb-1  | hello 2\n"
        );
        assert_eq!(
            colored(&stream),
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
            colored(b"\x1b[32m10%\x1b[0m\r\x1b[2K\x1b[32m100%\x1b[0m done\n"),
            "\x1b[32m100%\x1b[0m done\n"
        );
    }

    /// A title, a clipboard write, a query, a DCS, a scroll region, a mode
    /// change and an alternate screen leave nothing; the text beside them
    /// stays, and a line holding nothing else is dropped.
    #[test]
    fn sequences_that_steer_the_terminal_leave_nothing() {
        assert_eq!(
            colored(
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

    /// Colors outlive a line break as they do on a terminal, yet every line
    /// ends reset, and a bold red run reads back as one sequence.
    #[test]
    fn colors_carry_across_lines_but_never_leak() {
        assert_eq!(
            colored(b"\x1b[1m\x1b[31mred\nstill red\x1b[0m plain\n"),
            "\x1b[1;31mred\x1b[0m\n\x1b[1;31mstill red\x1b[0m plain\n"
        );
        assert_eq!(
            colored(b"\x1b[38;5;0mblack\x1b[0m\n"),
            "\x1b[38;5;0mblack\x1b[0m\n"
        );
        assert_eq!(colored(b"\x1b[31m   \n"), "\n");
    }

    /// A pen keeps only what is in effect: repeated, overridden and
    /// switched-off attributes leave nothing behind, so a stream that never
    /// resets cannot grow the sequence its characters are written with.
    #[test]
    fn a_pen_keeps_only_what_is_in_effect() {
        let mut repeated = b"\x1b[1mx".repeat(1000);
        repeated.push(b'\n');
        assert_eq!(
            colored(&repeated),
            format!("\x1b[1m{}\x1b[0m\n", "x".repeat(1000)),
            "one run, not one sequence per character"
        );
        assert_eq!(
            colored(b"\x1b[31m\x1b[32m\x1b[1;2m\x1b[22mgreen\n"),
            "\x1b[32mgreen\x1b[0m\n"
        );
        assert_eq!(colored(b"\x1b[4:3;9m\x1b[0;7mx\n"), "\x1b[7mx\x1b[0m\n");
        assert_eq!(colored(b"\x1b[4:3mx\n"), "\x1b[4:3mx\x1b[0m\n");
        assert_eq!(colored(b"\x1b[53;4;1mx\n"), "\x1b[1;4;53mx\x1b[0m\n");
    }

    /// Extended colors read the same in either form and are written back in
    /// one; a malformed one changes nothing.
    #[test]
    fn extended_colors_in_either_form() {
        assert_eq!(colored(b"\x1b[38:5:208mx\n"), "\x1b[38;5;208mx\x1b[0m\n");
        assert_eq!(
            colored(b"\x1b[48;2;1;2;3m\x1b[58:2::4:5:6mx\n"),
            "\x1b[48;2;1;2;3;58;2;4;5;6mx\x1b[0m\n"
        );
        assert_eq!(colored(b"\x1b[38;5;999m\x1b[38;2;1mx\n"), "x\n");
        assert_eq!(colored(b"\x1b[94;101m\x1b[49mx\n"), "\x1b[94mx\x1b[0m\n");
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

    /// A line shown in part goes on where it left off, and its line feed
    /// alone adds no blank line.
    #[test]
    fn a_line_shown_in_part_goes_on_where_it_left_off() {
        let mut renderer = LogRenderer::new();
        renderer.advance(b"Password: ");
        assert!(renderer.is_settled());
        assert_eq!(
            renderer.take_partial().map(|line| line.plain()),
            Some("Password:".to_string())
        );
        renderer.advance(b"hunter2\r\nnext\n\n");

        assert_eq!(taken(&mut renderer), vec![
            (" hunter2".to_string(), true),
            ("next".to_string(), false),
            (String::new(), false)
        ]);
        renderer.advance(b"Password: ");
        renderer.take_partial();
        renderer.advance(b"\r\n");
        assert!(taken(&mut renderer).is_empty());
    }

    /// Taking part of a line keeps its state, so a carriage return, a column
    /// move or an erase later on the same line lands where a terminal puts
    /// it, and the line is handed out again whole once the part shown
    /// changed.
    #[test]
    fn a_line_shown_in_part_keeps_its_columns() {
        let mut renderer = LogRenderer::new();
        renderer.advance(b"abcdef");
        renderer.take_partial();
        renderer.advance(b"\x1b[3GX\rY\n");

        assert_eq!(taken(&mut renderer), vec![("YbXdef".to_string(), false)]);
    }

    /// A line redrawn in place is not settled until its line feed, and the
    /// next line starts settled.
    #[test]
    fn a_redrawn_line_is_not_settled() {
        let mut renderer = LogRenderer::new();
        renderer.advance(b"    Building [=> ] 1/3: a\r");
        assert!(!renderer.is_settled());
        renderer.advance(b"\x1b[K   Compiling b\n    Building");
        assert!(renderer.is_settled());
    }

    /// Wide characters, combining marks and tabs land in the columns a
    /// terminal puts them in when a line is drawn over: overwriting half of
    /// a wide character blanks the rest of it, a combining mark goes with
    /// the character it was drawn onto, and a tab spans to its stop.
    #[test]
    fn glyphs_land_in_the_columns_a_terminal_gives_them() {
        assert_eq!(plain("你好世界\rab\n".as_bytes()), "ab好世界\n");
        assert_eq!(plain("你好\r\x1b[2Cx\n".as_bytes()), "你x\n");
        assert_eq!(plain("你好\x1b[4G!\n".as_bytes()), "你 !\n");
        assert_eq!(plain("e\u{301}e\rZ\n".as_bytes()), "Ze\n");
        assert_eq!(plain("cafe\u{301}\n".as_bytes()), "cafe\u{301}\n");
        assert_eq!(plain(b"tab\there\x1b[12Gx\n"), "tab\therx\n");
        assert_eq!(plain(b"abcdefghij\r\tX\n"), "abcdefghXj\n");
    }

    /// A move forward never takes the cursor back from a column beyond the
    /// furthest a move may reach.
    #[test]
    fn a_move_forward_never_moves_back() {
        let mut stream = vec![b'x'; MAX_COLUMN + 6];
        stream.extend_from_slice(b"\x1b[5Cy\n");

        let text = plain(&stream);

        assert_eq!(text, format!("{}y\n", "x".repeat(MAX_COLUMN + 6)));
        assert_eq!(
            plain(b"\x1b[5000Ca\x1b[5000Cb\n").trim_start(),
            "ab\n",
            "a move forward stops at the furthest column"
        );
    }

    /// A saved cursor that jumps away and is never restored ends the line
    /// drawn before the jump at the line feed, and a save never restored by
    /// the end of its line is forgotten, so a later absolute move, such as
    /// a screen clear, is not taken for a jump.
    #[test]
    fn saves_never_restored_lose_no_line() {
        assert_eq!(
            plain(b"progress 40%\x1b7\x1b[30;1Hstatus bar\nnext line\n"),
            "progress 40%\nnext line\n"
        );
        assert_eq!(
            plain(b"\x1b7saved here\n...\x1b[H\x1b[2JServer ready on :3000\n"),
            "saved here\nServer ready on :3000\n"
        );
    }

    /// A full reset (RIS) wipes the line it lands on and leaves the
    /// alternate screen and the pen behind.
    #[test]
    fn a_full_reset_starts_from_a_fresh_terminal() {
        assert_eq!(plain(b"before\x1bcafter\n"), "after\n");
        assert_eq!(
            colored(b"\x1b[31m\x1b[?1049hTUI\x1bcok\n"),
            "ok\n",
            "no alternate screen and no color after a reset"
        );
    }

    /// A control character never lands on a line, even when the parser
    /// hands a C1 control over as a printed character because it arrived in
    /// two pieces, and a format character lands as its escaped text, as it
    /// is shown everywhere else.
    #[test]
    fn controls_never_land_and_format_characters_are_escaped() {
        let mut renderer = LogRenderer::new();
        renderer.advance(b"x\xC2");
        renderer.advance(b"\x9B2J\n");
        renderer.advance(b"y\xC2");
        renderer.advance(b"\x9D52;c;aGk=\xC2");
        renderer.advance(b"\x9C\n");

        let lines: Vec<String> = renderer.take_lines().iter().map(LogLine::colored).collect();

        assert_eq!(lines, ["x2J", "y52;c;aGk="]);
        assert_eq!(
            plain("BIDI-\u{202e}txt.exe we\u{200b}b\n".as_bytes()),
            "BIDI-\\u{202e}txt.exe we\\u{200b}b\n"
        );
    }

    /// Erasing to the cursor costs the same however long the line is: a
    /// line of 64 Ki columns erased to its end again and again stays as
    /// cheap as a short one. The work is linear in the bytes fed, so the
    /// bound is far above what even a debug build needs.
    #[test]
    fn erasing_a_long_line_again_and_again_stays_cheap() {
        let mut renderer = LogRenderer::new();
        renderer.advance(&vec![b'x'; MAX_LINE_CELLS - 1]);
        let erases = b"\x1b[1K".repeat(ERASES);
        let started = Instant::now();

        renderer.advance(&erases);

        assert!(started.elapsed() < ERASE_BOUND, "{:?}", started.elapsed());
        assert_eq!(renderer.peek_partial(), None);
        renderer.advance(b"\rok");
        assert_eq!(
            renderer.peek_partial().map(|line| line.plain()).as_deref(),
            Some("ok")
        );
    }

    /// A restart forgets the alternate screen, the pen and a sequence cut
    /// short, so what follows renders as on a fresh terminal.
    #[test]
    fn a_restart_starts_from_a_fresh_terminal() {
        let mut renderer = LogRenderer::new();
        renderer.advance(b"run 1\n\x1b[?1049h\x1b[31mTUI\x1b]0;cut");
        renderer.restart();
        renderer.advance(b"run 2\n");

        let lines: Vec<String> = renderer.take_lines().iter().map(LogLine::colored).collect();

        assert_eq!(lines, ["run 1", "run 2"]);
    }
}
