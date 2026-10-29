//! The MCP server an agent launches over stdio: the project's stack
//! definition, its live processes through the muster service, the machine's
//! settings, and the shared notes, todos and key-values.

mod config_get_arg;
mod config_set_arg;
mod key_arg;
mod kv_set_arg;
mod list_arg;
mod mcp_deps;
mod mcp_error;
mod name_arg;
mod nullable;
mod process_add_arg;
mod process_fields;
mod process_set_arg;
mod scratchpad_entry;
mod scratchpad_set_arg;
mod send_input_arg;
mod server;
mod todo_add_arg;
mod todo_id_arg;
mod todo_status_arg;
mod tool_error;
mod tool_failure;

pub use config_get_arg::*;
pub use config_set_arg::*;
pub use key_arg::*;
pub use kv_set_arg::*;
pub use list_arg::*;
pub use mcp_deps::*;
pub use mcp_error::*;
pub use name_arg::*;
pub use process_add_arg::*;
pub use process_set_arg::*;
pub use scratchpad_entry::*;
pub use scratchpad_set_arg::*;
pub use send_input_arg::*;
pub use server::*;
pub use todo_add_arg::*;
pub use todo_id_arg::*;
pub use todo_status_arg::*;
pub use tool_error::*;
pub use tool_failure::*;
