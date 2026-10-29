//! Terminal-side pieces the supervisor and holder keep for every process: the
//! VT emulator and the keyboard-protocol tracker. None of them depends on a
//! rendering frontend.

pub mod cursor_state;
pub mod emulator;
pub mod keyboard_protocol;
pub mod keyboard_tracker;
pub mod mode_scanner;
pub mod screen_snapshot;
pub mod tab_stops;

pub use emulator::TerminalEmulator;
pub use keyboard_tracker::KittyKeyboardTracker;
