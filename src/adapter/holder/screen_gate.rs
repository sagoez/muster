use crate::adapter::terminal::{
    escape::{END_OPEN_LINE, ESC},
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
    /// the emulator found `switches`. The sequence that brings the alternate
    /// screen up starts at the last escape before its switch; one that began
    /// in an earlier chunk is cancelled by the line ending
    /// [`FULL_SCREEN_LINE`] starts with.
    pub fn logged(&mut self, bytes: &[u8], switches: &[ScreenSwitch]) -> Vec<u8> {
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
                    logged.extend_from_slice(END_OPEN_LINE);
                    logged.extend_from_slice(FULL_SCREEN_LINE.as_bytes());
                    logged.extend_from_slice(LINE_BREAK);
                }
            }
            self.alternate = switch.alternate();
            from = at;
        }
        if !self.alternate {
            logged.extend_from_slice(&bytes[from..]);
        }
        logged
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        adapter::terminal::{TerminalEmulator, coloring::Coloring, log_renderer::LogRenderer},
        domain::{
            color_scheme::ColorScheme,
            value::{Cols, Rows},
        },
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
            log.extend(gate.logged(chunk, &emulator.take_screen_switches()));
        }
        log
    }

    /// The lines a log of `bytes` reads as.
    fn read(bytes: &[u8]) -> Vec<String> {
        let mut renderer = LogRenderer::new();
        renderer.advance(bytes);
        renderer.finish();
        renderer
            .take_lines()
            .iter()
            .map(|line| line.text(Coloring::Plain))
            .collect()
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
