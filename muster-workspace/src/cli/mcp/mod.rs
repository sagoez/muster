//! `muster mcp`: serving the MCP server, setting it up with an agent
//! through the agent's own plugin commands, the context the plugin's
//! session-start hook prints, and the guard its tool hooks run.

pub(crate) mod context;
pub(crate) mod guard;
pub(crate) mod mcp_command;
pub(crate) mod mcp_launch;
pub(crate) mod registration;

pub use context::{bounds::CONTEXT_MAX_BYTES, session_context::CONTEXT_DEADLINE};
pub use mcp_launch::*;
pub use registration::mcp_register::{MARKETPLACE_REPO, PLUGIN_ID, RELEASE_BRANCH};
