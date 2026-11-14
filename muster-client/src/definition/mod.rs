//! A process's stored definition as `--json` and MCP show it.

mod definition_view;
mod environment_view;
mod ready_view;
mod removed_view;
mod stop_view;

pub use definition_view::*;
pub use environment_view::*;
pub use ready_view::*;
pub use removed_view::*;
pub use stop_view::*;
