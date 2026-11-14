//! A pane's screen, wherever it is served, and the clients attached to it.

pub(crate) mod attach_book;
mod bridge;
mod holder_screen;
mod screen_reply;

pub use holder_screen::*;
pub use screen_reply::*;
