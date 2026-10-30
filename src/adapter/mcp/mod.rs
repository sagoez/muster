//! The MCP server an agent launches over stdio: each project's stack
//! definition, its live processes through the muster service, the machine's
//! settings, and the shared notes, todos, key-values and locks. Each call
//! resolves its own project.

mod agent_wait_view;
mod alerts;
mod alerts_view;
mod blocked_view;
mod bounds;
mod condition_arg;
mod config_get_arg;
mod config_set_arg;
mod config_tools;
mod control_arg;
mod control_view;
mod coordination_tools;
mod described;
mod event_view;
mod events_view;
mod identity;
mod key_arg;
mod kv_write_arg;
mod last_exit_view;
mod lock_acquire_arg;
mod lock_name_arg;
mod mcp_deps;
mod mcp_error;
mod name_arg;
mod note_write_arg;
mod occupancy_view;
mod process_add_arg;
mod process_fields;
mod process_logs_arg;
mod process_set_arg;
mod process_tools;
mod process_wait_arg;
mod progress;
mod project_arg;
mod read_arg;
mod ready_arg;
mod rendering;
mod scope;
mod seconds_arg;
mod send_input_arg;
mod server;
mod stack_events_arg;
mod stack_process_view;
mod stack_status_view;
mod stack_tools;
mod stack_up_arg;
mod stdio_runtime;
#[cfg(test)]
mod test_support;
mod todo_add_arg;
mod todo_claim_arg;
mod todo_id_arg;
mod todo_list_arg;
mod todo_update_arg;
mod tool_error;
mod tool_failure;
mod tool_result;
mod up_view;
mod wait_arg;

pub use mcp_deps::*;
pub use mcp_error::*;
pub use scope::*;
pub use server::{MusterMcp, connection_config};
pub use stdio_runtime::*;
pub use tool_error::*;
pub use tool_failure::*;
