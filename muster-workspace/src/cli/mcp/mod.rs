//! `muster mcp`: serving the MCP server, setting it up with an agent
//! through the agent's own plugin commands, and the context the plugin's
//! session-start hook prints.

mod context;
pub(crate) mod mcp_command;
pub(crate) mod mcp_launch;
mod registration;

pub use context::{CONTEXT_DEADLINE, CONTEXT_MAX_BYTES};
pub(crate) use context::{listing, rendering, session_context};
pub use mcp_launch::*;
pub use registration::mcp_register::{MARKETPLACE_SOURCE, PLUGIN_ID};
pub(crate) use registration::{mcp_agent, mcp_register, mcp_registration, plugin_change};
