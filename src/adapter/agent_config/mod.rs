//! Driven adapter: the MCP server entries of agents' own config files
//! (Claude Code, Codex, Zed), added and removed with every other key,
//! comment and blank line of the file kept as written.

mod agent;
mod agent_config_error;
mod agent_config_files;
mod config_locations;
mod config_target;
mod edit_outcome;
mod edited;
mod entry_change;
mod json_document;
mod mcp_registry;
mod mcp_scope;
mod planned_edit;
mod server_entry;
mod server_launch;
mod server_name;
mod toml_document;

pub use agent::*;
pub use agent_config_error::*;
pub use agent_config_files::*;
pub use config_locations::*;
pub use config_target::*;
pub use edit_outcome::*;
pub use entry_change::*;
pub use mcp_registry::*;
pub use mcp_scope::*;
pub use planned_edit::PlannedEdit;
pub use server_launch::*;
pub use server_name::*;
