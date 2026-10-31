//! Driven adapter: the MCP server entries of agents' own config files
//! (Claude Code, Codex, Zed), added and removed with every other key,
//! comment and blank line of the file kept as written.

mod agent;
mod agent_config_error;
mod agent_config_files;
mod claude_project_key;
mod config_file;
mod config_format;
mod config_locations;
mod config_lock;
mod config_place;
mod config_target;
mod edit_outcome;
mod edit_plan;
mod edited;
mod entry_action;
mod entry_change;
mod entry_match;
mod file_edit;
mod file_fingerprint;
mod json_dialect;
mod json_document;
mod json_place;
mod lock_timing;
mod mcp_registry;
mod mcp_scope;
mod planned_edit;
mod server_entry;
mod server_launch;
mod server_name;
#[cfg(test)]
pub(crate) mod testing;
mod toml_document;

pub use agent::*;
pub use agent_config_error::*;
pub use agent_config_files::*;
pub use config_locations::*;
pub use config_target::*;
pub use edit_outcome::*;
pub use edit_plan::*;
pub use entry_change::*;
pub use mcp_registry::*;
pub use mcp_scope::*;
pub use planned_edit::PlannedEdit;
pub use server_launch::*;
pub use server_name::*;
