//! Terminal-side pieces the supervisor and holder keep for every process: the
//! VT emulator and the keyboard-protocol tracker. None of them depends on a
//! rendering frontend.

pub mod bounded_parser;
pub mod cursor_state;
pub mod emulator;
pub mod escape;
pub mod fed;
pub mod keyboard_protocol;
pub mod keyboard_tracker;
pub mod mode_scanner;
pub mod screen_snapshot;
pub mod tab_stops;
mod watched_performer;

pub use emulator::TerminalEmulator;
pub use keyboard_tracker::KittyKeyboardTracker;
