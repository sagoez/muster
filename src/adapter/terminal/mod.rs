//! Terminal-side pieces: the VT emulator and keyboard-protocol tracker the
//! holder keeps for every process, and the log line renderer behind every
//! read of a process's log, the service's waits and the CLI's `logs`.
//! None of them depends on a rendering frontend.

pub mod bounded_parser;
pub mod cell_color;
pub mod cell_flags;
pub mod cell_style;
pub mod coloring;
pub mod cursor_state;
#[cfg(test)]
mod dispatch_recorder;
pub mod emulator;
mod emulator_sink;
mod erasure;
pub mod escape;
mod event_proxy;
pub mod fed;
#[cfg(test)]
pub mod fuzz_stream;
mod keyboard;
pub mod keyboard_protocol;
pub mod keyboard_tracker;
pub mod line_cell;
pub mod line_drawing;
pub mod log_line;
pub mod log_renderer;
pub mod mode_scanner;
pub mod mouse_protocol_encoding;
pub mod mouse_protocol_mode;
pub mod observation;
mod param_separator;
pub mod pen;
pub mod pen_color;
pub mod placed_renderer;
#[cfg(test)]
pub mod render_cell;
#[cfg(test)]
pub mod render_fault;
mod saved_cursor;
mod scanned_modes;
pub mod screen_snapshot;
mod screen_state;
pub mod snapshot_cell;
pub mod tab_stops;
mod watched_performer;

pub use emulator::TerminalEmulator;
pub use keyboard_tracker::KittyKeyboardTracker;
