use std::path::PathBuf;

use bytesize::ByteSize as RawByteSize;

use super::{log::RotatingLog, screen_gate::ScreenGate};
use crate::{
    adapter::terminal::TerminalEmulator,
    domain::{
        color_scheme::ColorScheme,
        value::{ByteSize, Cols, OutputCursor, Rows},
    },
};

/// Screen rows of the emulator a [`GatedLog`] follows the screen with.
const ROWS: u16 = 24;
/// Screen columns of that emulator.
const COLS: u16 = 80;

/// A log written as a holder writes one: each chunk through an emulator
/// and the [`ScreenGate`], so a test reads what a holder would have logged.
pub(crate) struct GatedLog {
    /// Follows the screen the chunks draw on.
    emulator: TerminalEmulator,
    /// Decides what of each chunk is logged.
    gate: ScreenGate,
    /// The log.
    log: RotatingLog,
}

impl GatedLog {
    /// A log at `path` rotating at `cap` bytes.
    pub(crate) fn open(path: PathBuf, cap: u64) -> Self {
        Self {
            emulator: screen(),
            gate: ScreenGate::default(),
            log: RotatingLog::open(path, ByteSize::try_new(RawByteSize::b(cap)).unwrap()).unwrap(),
        }
    }

    /// Begins a run, as a new holder does before its child writes, on a
    /// fresh screen; returns where the run starts.
    pub(crate) fn start_run(&mut self) -> OutputCursor {
        self.emulator = screen();
        self.gate = ScreenGate::default();
        self.log.start_run();
        self.log.run_start()
    }

    /// Logs what a holder keeps of `chunk`, and returns where that went and
    /// what it was: what the holder sends its supervisor.
    pub(crate) fn write(&mut self, chunk: &[u8]) -> (OutputCursor, Vec<u8>) {
        self.emulator.process(chunk);
        let logged = self
            .gate
            .logged(chunk, &self.emulator.take_screen_switches());
        let at = self.log.end();
        let (held, outcome) = self.log.append_held(logged);
        outcome.unwrap();
        (at, held.into_owned())
    }

    /// Where the log ends.
    pub(crate) fn end(&self) -> OutputCursor {
        self.log.end()
    }
}

/// A fresh screen for a run's output.
fn screen() -> TerminalEmulator {
    TerminalEmulator::new(
        Rows::try_new(ROWS).unwrap(),
        Cols::try_new(COLS).unwrap(),
        0,
        ColorScheme::Dark,
    )
}
