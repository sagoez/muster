//! Terminal-side pieces the supervisor and holder keep for every process: the
//! VT emulator and the keyboard-protocol tracker. None of them depends on a
//! rendering frontend.

pub mod bounded_parser;
pub mod cell_color;
pub mod cell_flags;
pub mod cell_style;
pub mod cursor_state;
#[cfg(test)]
mod dispatch_recorder;
pub mod emulator;
mod emulator_sink;
pub mod escape;
mod event_proxy;
pub mod fed;
mod keyboard;
pub mod keyboard_protocol;
pub mod keyboard_tracker;
pub mod mode_scanner;
pub mod mouse_protocol_encoding;
pub mod mouse_protocol_mode;
pub mod observation;
#[cfg(test)]
pub mod render_cell;
mod scanned_modes;
pub mod screen_snapshot;
mod screen_state;
pub mod snapshot_cell;
pub mod tab_stops;
mod watched_performer;

pub use emulator::TerminalEmulator;
pub use keyboard_tracker::KittyKeyboardTracker;
