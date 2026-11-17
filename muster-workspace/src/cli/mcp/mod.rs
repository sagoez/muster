//! `muster mcp`: serving the MCP server, setting it up with an agent
//! through the agent's own plugin commands, and the context the plugin's
//! session-start hook prints.

pub(crate) mod context;
pub(crate) mod mcp_command;
pub(crate) mod mcp_launch;
pub(crate) mod registration;

pub use context::{CONTEXT_DEADLINE, CONTEXT_MAX_BYTES};
pub use mcp_launch::*;
pub use registration::mcp_register::{MARKETPLACE_REPO, PLUGIN_ID};
