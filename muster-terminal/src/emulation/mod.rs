//! The emulator over `alacritty_terminal` and what its mode scanner reports.

pub mod emulator;
mod emulator_sink;
mod event_proxy;
pub mod mode_scanner;
pub mod mouse_protocol_encoding;
pub mod mouse_protocol_mode;
pub mod observation;
mod scanned_modes;
pub mod screen_switch;
pub mod tab_stops;
