//! `muster mcp`: serving the MCP server, registering it with an agent along
//! with the session-start hook, and the context that hook prints.

pub(crate) mod agent_keys;
mod agent_output;
pub(crate) mod claude_config;
pub(crate) mod hook_command;
mod hook_edit;
pub(crate) mod hook_file;
pub(crate) mod hook_file_error;
mod hook_file_part;
mod json_shape;
pub(crate) mod mcp_agent;
pub(crate) mod mcp_command;
pub(crate) mod mcp_launch;
pub(crate) mod mcp_register;
mod mcp_registration;
pub(crate) mod mcp_scope;
pub(crate) mod session_context;

pub use hook_file::{HOOK_TIMEOUT_S, SESSION_SOURCES};
pub use mcp_launch::*;
pub use session_context::{CONTEXT_DEADLINE, CONTEXT_MAX_BYTES};
