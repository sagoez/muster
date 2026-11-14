//! What the CLI and the MCP server share to drive the muster service: the
//! service client, the process use cases and the reports and views they
//! answer with. Each driver maps [`ClientError`] to its own surface.

mod connect;
mod coordination;
mod definition;
mod failure;
mod lifecycle;
mod logs;
mod notices;
mod pane;
mod project;
mod settings;
mod startup;
mod state_change;
mod status;
mod wait;

pub use connect::*;
pub use coordination::*;
pub use definition::*;
pub use failure::*;
pub use lifecycle::*;
pub use logs::*;
pub use notices::*;
pub use pane::*;
pub use project::*;
pub use settings::*;
pub use startup::*;
pub use state_change::*;
pub use status::*;
pub use wait::*;
