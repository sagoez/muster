//! Stand-ins for the interfaces the parallel control-plane packages land:
//! the log reader (A), waits and events (B), claims, locks and identity (C).
//! Each mirrors its design signature, so integration deletes this module
//! and points the imports at the real items.

mod coordination;
mod events;
mod identity;
mod log_reader;
mod output_cursor;
mod wait;

pub use coordination::*;
pub use events::*;
pub use identity::*;
pub use log_reader::*;
pub use output_cursor::*;
pub use wait::*;
