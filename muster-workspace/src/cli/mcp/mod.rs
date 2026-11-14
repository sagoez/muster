//! `muster mcp`: serving the MCP server, and registering it with an agent.

pub(crate) mod mcp_agent;
pub(crate) mod mcp_command;
pub(crate) mod mcp_launch;
pub(crate) mod mcp_register;
mod mcp_registration;
pub(crate) mod mcp_scope;

pub use mcp_launch::*;
