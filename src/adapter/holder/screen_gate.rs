use std::borrow::Cow;

use crate::adapter::terminal::{
    escape::{END_OPEN_LINE, ESC, POP_PEN, PUSH_PEN, SGR_RESET},
    screen_switch::ScreenSwitch,
};

/// The line a log holds in place of what a child drew on the alternate
/// screen.
pub const FULL_SCREEN_LINE: &str = "[full-screen output not logged]";
/// Ends [`FULL_SCREEN_LINE`] in the log.
const LINE_BREAK: &[u8] = b"\r\n";

/// Decides what of a child's output its log keeps: everything drawn on the
/// primary screen, and in place of a full-screen program's frames, one
/// [`FULL_SCREEN_LINE`] each time the alternate screen comes up. No byte
/// drawn on the alternate screen, nor the sequence that brings it up or
/// takes it away, reaches the log, so a reader starting anywhere in it needs
/// to know nothing of the screen.
#[derive(Debug, Default)]
pub struct ScreenGate {
    /// Whether the alternate screen is up where the last chunk ended.
    alternate: bool,
}

impl ScreenGate {
    /// What the log keeps of `bytes`, the next chunk of output, in which
    /// the emulator found `switches`: `bytes` itself, or nothing, when the
    /// chunk switches no screen. The sequence that brings the alternate
    /// screen up starts at the last escape before its switch; one that began
    /// in an earlier chunk is cancelled by the line ending
    /// [`FULL_SCREEN_LINE`] starts with. That line is drawn in a reset pen
    /// pushed and popped around it, so it takes no color from the output
    /// before it and leaves the pen as it found it, whichever mode brought
    /// the screen up.
    pub fn logged<'a>(&mut self, bytes: &'a [u8], switches: &[ScreenSwitch]) -> Cow<'a, [u8]> {
        if switches.is_empty() {
            return Cow::Borrowed(if self.alternate { &[] } else { bytes });
        }
        let mut logged = Vec::with_capacity(bytes.len());
        let mut from = 0;
        for switch in switches {
            let at = switch.at().clamp(from, bytes.len());
            if !self.alternate {
                let kept = if switch.alternate() {
                    bytes[from..at]
                        .iter()
                        .rposition(|byte| *byte == ESC)
                        .map_or(from, |escape| from + escape)
                } else {
                    at
                };
                logged.extend_from_slice(&bytes[from..kept]);
                if switch.alternate() {
                    for part in [
                        END_OPEN_LINE,
                        PUSH_PEN,
                        SGR_RESET.as_bytes(),
                        FULL_SCREEN_LINE.as_bytes(),
                        POP_PEN,
                        LINE_BREAK,
                    ] {
                        logged.extend_from_slice(part);
                    }
                }
            }
            self.alternate = switch.alternate();
            from = at;
        }
        if !self.alternate {
            logged.extend_from_slice(&bytes[from..]);
        }
        Cow::Owned(logged)
    }
}

#[cfg(test)]
mod tests {
    use muster_domain::{
        color_scheme::ColorScheme,
        value::{Cols, Rows},
    };

    use super::*;
    use crate::adapter::terminal::{
        TerminalEmulator, coloring::Coloring, log_renderer::LogRenderer,
    };

    /// Screen rows of the emulator the tests feed.
    const ROWS: u16 = 4;
    /// Screen columns of that emulator.
    const COLS: u16 = 40;

    /// What a log keeps of `chunks`, fed in turn through one emulator.
    fn logged(chunks: &[&[u8]]) -> Vec<u8> {
        let mut emulator = TerminalEmulator::new(
            Rows::try_new(ROWS).unwrap(),
            Cols::try_new(COLS).unwrap(),
            0,
            ColorScheme::Dark,
        );
        let mut gate = ScreenGate::default();
        let mut log = Vec::new();
        for chunk in chunks {
            emulator.process(chunk);
            log.extend_from_slice(&gate.logged(chunk, &emulator.take_screen_switches()));
        }
        log
    }

    /// The lines a log of `bytes` reads as, colored as `coloring` asks.
    fn read_as(bytes: &[u8], coloring: Coloring) -> Vec<String> {
        let mut renderer = LogRenderer::new();
        renderer.advance(bytes);
        renderer.finish();
        renderer
            .take_lines()
            .iter()
            .map(|line| line.text(coloring))
            .collect()
    }

    /// The lines a log of `bytes` reads as.
    fn read(bytes: &[u8]) -> Vec<String> {
        read_as(bytes, Coloring::Plain)
    }

    /// A chunk that switches no screen is logged as it came, without a
    /// copy, and one wholly on the alternate screen leaves nothing.
    #[test]
    fn a_chunk_without_a_switch_is_passed_through() {
        let mut gate = ScreenGate::default();
        let plain = b"plain\r\n".as_slice();

        assert!(matches!(gate.logged(plain, &[]), Cow::Borrowed(kept) if kept == plain));
        gate.alternate = true;
        assert!(matches!(gate.logged(plain, &[]), Cow::Borrowed(kept) if kept.is_empty()));
    }

    /// The line in place of a full-screen run is drawn in a reset pen
    /// whatever color was set before it, and the output after it is drawn
    /// in the pen it had before, the same for every mode that brings the
    /// alternate screen up.
    #[test]
    fn the_full_screen_line_takes_and_leaves_no_color() {
        let colored = |mode: &str| {
            let enter = format!("\x1b[31mred\r\n\x1b[?{mode}h\x1b[32mframe");
            let leave = format!("\x1b[?{mode}lstill red\r\n");
            read_as(
                &logged(&[enter.as_bytes(), leave.as_bytes()]),
                Coloring::Colored,
            )
        };

        let expected = [
            "\x1b[31mred\x1b[0m".to_string(),
            FULL_SCREEN_LINE.to_string(),
            "\x1b[31mstill red\x1b[0m".to_string(),
        ];
        for mode in ["47", "1047", "1049"] {
            assert_eq!(colored(mode), expected, "mode {mode}");
        }
    }

    /// A full-screen program leaves one line in the log, however many frames
    /// it draws and wherever its switches fall in a chunk, and the output
    /// around it is kept whole.
    #[test]
    fn a_full_screen_run_leaves_one_line() {
        let log = logged(&[
            b"before\r\n\x1b[?1049h\x1b[Hframe 1\r\n",
            b"frame 2\x1b[?1049lafter\r\n\x1b[?1049hframe 3\x1b[?1049l",
            b"end\r\n",
        ]);

        assert!(!String::from_utf8_lossy(&log).contains("frame"));
        assert!(!log.windows(2).any(|pair| pair == b"[?"));
        assert_eq!(read(&log), [
            "before",
            FULL_SCREEN_LINE,
            "after",
            FULL_SCREEN_LINE,
            "end"
        ]);
    }

    /// A switch split across chunks leaves nothing of its sequence in the
    /// log that a reader could act on, a chunk wholly on the alternate
    /// screen leaves nothing, and a line left open before it is ended.
    #[test]
    fn a_split_switch_and_an_open_line_read_cleanly() {
        let log = logged(&[
            b"\x1b[31mprompt> \x1b[?10",
            b"49h",
            b"\x1b[2J frame",
            b"\x1b[?1049",
            b"lok\r\n",
        ]);

        assert_eq!(read(&log), ["prompt>", FULL_SCREEN_LINE, "ok"]);
    }

    /// A full reset on the alternate screen takes it away, as a leave does.
    #[test]
    fn a_full_reset_ends_the_full_screen_run() {
        let log = logged(&[b"\x1b[?47hTUI\x1bcback\r\n"]);

        assert_eq!(read(&log), [FULL_SCREEN_LINE, "back"]);
    }
}
