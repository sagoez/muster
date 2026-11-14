//! A project and its processes as `muster ps --json` and MCP show them.

pub(crate) mod elapsed_seconds;
mod exit_view;
pub mod idle_stop_seconds;
mod occupancy_view;
pub(crate) mod port_block_view;
mod port_owner_view;
pub(crate) mod process_view;
pub(crate) mod project_identity_view;
mod project_view;

pub use elapsed_seconds::*;
pub use exit_view::*;
pub use occupancy_view::*;
pub use port_block_view::*;
pub use port_owner_view::*;
pub use process_view::*;
pub use project_identity_view::*;
pub use project_view::*;
