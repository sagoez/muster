//! Panes and their terminals: ids, sizes, scrollback, text and the detach
//! key.

mod cols;
mod detach_key;
mod pane_id;
mod rows;
mod scrollback_lines;
mod terminal_text;
mod workspace_pane_id;

pub use cols::*;
pub use detach_key::*;
pub use pane_id::*;
pub use rows::*;
pub use scrollback_lines::*;
pub use terminal_text::*;
pub use workspace_pane_id::*;
