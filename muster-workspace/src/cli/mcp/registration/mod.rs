//! `muster mcp add` and `remove`: the agents, their scopes and config, and
//! the registration with its rollback.

pub(crate) mod agent_output;
pub(crate) mod claude_config;
pub(crate) mod codex_config;
pub(crate) mod codex_entry;
pub(crate) mod deferred_signals;
pub(crate) mod mcp_agent;
pub(crate) mod mcp_register;
pub(crate) mod mcp_registration;
pub(crate) mod mcp_scope;
