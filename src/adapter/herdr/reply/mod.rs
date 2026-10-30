//! The shapes of herdr's JSON answers muster reads, deserialize-only. Only
//! the three whole answers leave this module; the rest are their parts.

mod error_body;
mod error_reply;
mod layout;
mod layout_pane;
mod layout_reply;
mod layout_result;
mod pane_info;
mod pane_rect;
mod pane_reply;
mod pane_result;

pub use error_reply::*;
pub use layout_reply::*;
pub use pane_reply::*;
