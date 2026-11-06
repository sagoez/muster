//! A terminal workspace the caller may run in, which can open a pane beside
//! the caller's: what is asked of it and what it answers.

mod opened_pane;
mod pane_command;
mod pane_focus;
mod pane_left;
mod pane_request;
mod split_direction;
mod workspace_error;
mod workspace_probe;

pub use opened_pane::*;
pub use pane_command::*;
pub use pane_focus::*;
pub use pane_left::*;
pub use pane_request::*;
pub use split_direction::*;
pub use workspace_error::*;
pub use workspace_probe::*;
