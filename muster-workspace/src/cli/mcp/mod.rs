//! `muster mcp`: serving the MCP server, registering it with an agent along
//! with the session-start hook, and the context that hook prints.

pub(crate) mod hook_file;
pub(crate) mod hook_file_error;
pub(crate) mod mcp_agent;
pub(crate) mod mcp_command;
pub(crate) mod mcp_launch;
pub(crate) mod mcp_register;
mod mcp_registration;
pub(crate) mod mcp_scope;
pub(crate) mod session_context;

pub use hook_file::SESSION_SOURCES;
pub use mcp_launch::*;
pub use session_context::CONTEXT_MAX_CHARS;
