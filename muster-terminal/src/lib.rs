//! Terminal-side pieces: the VT emulator and keyboard-protocol tracker the
//! holder keeps for every process, and the log line renderer behind every
//! read of a process's log, the service's waits and the CLI's `logs`.
//! None of them depends on a rendering frontend.

mod drawing;
mod emulation;
mod keys;
mod log;
mod parser;
mod snapshot;

pub use drawing::{line_cell, line_drawing, pen, pen_color};
pub use emulation::{
    emulator, mode_scanner, mouse_protocol_encoding, mouse_protocol_mode, observation,
    screen_switch, tab_stops,
};
pub use emulator::TerminalEmulator;
pub use keyboard_tracker::KittyKeyboardTracker;
pub use keys::{keyboard_protocol, keyboard_tracker};
#[cfg(test)]
pub use log::fuzz_stream;
#[cfg(any(test, feature = "testing"))]
pub use log::render_fault;
pub use log::{coloring, log_line, log_renderer, placed_renderer};
pub use parser::{bounded_parser, escape, fed};
#[cfg(test)]
pub use snapshot::render_cell;
pub use snapshot::{
    cell_color, cell_flags, cell_style, cursor_state, screen_snapshot, snapshot_cell,
};
