//! `muster mcp`: serving the MCP server, registering it with an agent along
//! with the session-start hook, and the context that hook prints.

mod agent_files;
mod context;
pub(crate) mod mcp_command;
pub(crate) mod mcp_launch;
mod registration;

pub use agent_files::{HOOK_TIMEOUT_S, SESSION_SOURCES};
pub(crate) use agent_files::{
    agent_file, agent_keys, hook_command, hook_file, hook_file_error, hook_file_part, json_shape,
};
pub use context::{CONTEXT_DEADLINE, CONTEXT_MAX_BYTES};
pub(crate) use context::{hook_input, session_context, session_marks};
pub use mcp_launch::*;
pub(crate) use registration::{
    claude_config, codex_config, mcp_agent, mcp_register, mcp_registration, mcp_scope,
};
