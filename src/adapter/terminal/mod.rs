//! Terminal-side pieces the supervisor keeps for every pane: the VT emulator,
//! the out-of-band signal reader, the keyboard-protocol tracker, and the
//! activity tracker. None of them depends on a rendering frontend.

pub mod activity;
pub mod cursor_state;
pub mod emulator;
pub mod keyboard_protocol;
pub mod keyboard_tracker;
pub mod mode_scanner;
pub mod screen_snapshot;
pub mod signal;
pub mod tab_stops;

pub use activity::ActivityTracker;
pub use emulator::{SCROLLBACK_LINES, TerminalEmulator};
pub use keyboard_tracker::KittyKeyboardTracker;
pub use signal::{Signal, SignalReader};
