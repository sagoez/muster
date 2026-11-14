//! The detach key and the key reports typed into a session.

pub(crate) mod detach_scanner;
mod held_sequence;
mod key_press;
mod parsed;
mod scan_result;

pub use detach_scanner::*;
pub use scan_result::*;
