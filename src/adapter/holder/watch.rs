use std::time::Instant;

use crossbeam_channel::{Receiver, unbounded};

use super::{
    server_testing::WAIT,
    testing::{TEST_COLS, TEST_ROWS, TEST_SCROLLBACK},
};
use crate::{
    adapter::{channel_sink::ChannelSink, terminal::TerminalEmulator},
    domain::{
        color_scheme::ColorScheme,
        port::{OutputSink, ProcessHandle},
        pty::{ExitOutcome, ProcessOutput},
        value::{Cols, Rows},
    },
};

/// What a sink received: each chunk, the exit, and the screen they draw.
pub(super) struct Watch {
    outputs: Receiver<ProcessOutput>,
    chunks: Vec<Vec<u8>>,
    exit: Option<ExitOutcome>,
    screen: TerminalEmulator,
}

impl Watch {
    /// A sink to spawn with, and the watch over what it receives.
    pub(super) fn new() -> (Box<dyn OutputSink>, Self) {
        let (sender, outputs) = unbounded();
        let watch = Self {
            outputs,
            chunks: Vec::new(),
            exit: None,
            screen: TerminalEmulator::new(
                Rows::try_new(TEST_ROWS).unwrap(),
                Cols::try_new(TEST_COLS).unwrap(),
                TEST_SCROLLBACK,
                ColorScheme::Dark,
            ),
        };
        (Box::new(ChannelSink::new(sender)), watch)
    }

    /// The exit received, if any.
    pub(super) fn exit(&self) -> Option<ExitOutcome> {
        self.exit.clone()
    }

    /// Every chunk received, in order.
    pub(super) fn chunks(&self) -> &[Vec<u8>] {
        &self.chunks
    }

    /// The screen the received output draws.
    pub(super) fn screen(&self) -> &TerminalEmulator {
        &self.screen
    }

    /// Receives until `condition` holds, failing the test at [`WAIT`].
    pub(super) fn until(&mut self, what: &str, condition: impl Fn(&Self) -> bool) {
        let deadline = Instant::now() + WAIT;
        while !condition(self) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.outputs.recv_timeout(remaining) {
                Ok(output) => self.apply(output),
                Err(_) => panic!("timed out waiting for {what}"),
            }
        }
    }

    /// Applies everything received so far without waiting.
    pub(super) fn drain(&mut self) {
        while let Ok(output) = self.outputs.try_recv() {
            self.apply(output);
        }
    }

    /// Records one output: its bytes onto the screen, or the exit.
    fn apply(&mut self, output: ProcessOutput) {
        match output {
            ProcessOutput::Chunk(bytes)
            | ProcessOutput::Placed { bytes, .. }
            | ProcessOutput::Repaint { bytes, .. } => {
                self.screen.process(&bytes);
                self.chunks.push(bytes);
            },
            ProcessOutput::Exited(outcome) => self.exit = Some(outcome),
            ProcessOutput::Warning(_) | ProcessOutput::Attached(_) => {},
            ProcessOutput::Unreadable(reason) => panic!("unreadable output: {reason}"),
        }
    }

    /// Every byte received so far, as text.
    pub(super) fn text(&self) -> String {
        String::from_utf8_lossy(&self.chunks.concat()).into_owned()
    }

    /// Paints the screen `handle` connected with first, as the service does,
    /// since it is handed back rather than sent to the sink, then takes its
    /// backlog as the service does, which lets an adopting handle replay.
    pub(super) fn connected(&mut self, handle: &mut Box<dyn ProcessHandle>) {
        let screen = handle
            .take_initial_screen()
            .expect("a holder connection carries its screen");
        self.apply(ProcessOutput::Chunk(screen));
        let _ = handle.take_backlog();
    }
}
