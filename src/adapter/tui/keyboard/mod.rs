//! Keyboard input encoding, organized by the protocol a PTY child speaks.
//!
//! muster relays keystrokes from the host terminal to each child process. A
//! child negotiates a keyboard protocol on its output stream;
//! [`KittyKeyboardTracker`] watches for that negotiation (and answers capability
//! queries), reporting the level as a [`KeyboardProtocol`]; [`encode_key`] then
//! renders each key the way the negotiated protocol expects.
//!
//! Two protocols are supported:
//! - **Legacy** ([`legacy`]): the traditional escape sequences every child
//!   understands.
//! - **Kitty** ([`kitty`]): the progressive-enhancement protocol
//!   (github.com/kovidgoyal/kitty), which disambiguates keys such as Shift+Enter,
//!   Ctrl+Backspace, and a bare Escape that the legacy encoding collapses.
//!
//! Other disambiguation protocols, notably xterm's `modifyOtherKeys`, are
//! **intentionally not supported**. muster reads host input through crossterm,
//! which speaks the Kitty protocol natively; honoring `modifyOtherKeys` would
//! mean replacing crossterm's input reader with a bespoke raw parser (as herdr
//! does). The Kitty protocol covers the modern terminals agent CLIs run in, and
//! muster degrades cleanly to Legacy everywhere else, so a second input stack is
//! not worth its weight. The matching host-side capability request lives in
//! [`crate::adapter::tui::terminal`].
//!
//! The Kitty encoding is ported from herdr (github.com/ogulcancelik/herdr,
//! `src/input/encode.rs`); credit for the protocol handling belongs there.

mod encode;
mod key;
mod legacy;
mod protocol;
mod tracker;

pub use encode::{encode_held, encode_key};
pub use key::{HeldEncoding, held_encoding};
pub use protocol::KeyboardProtocol;
pub use tracker::KittyKeyboardTracker;
