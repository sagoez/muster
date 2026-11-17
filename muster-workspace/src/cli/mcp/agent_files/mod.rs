//! The agents' config files muster edits for its hook: reading and writing
//! them, the hooks file and its edits, and the hook's command line.

pub(crate) mod agent_file;
pub(crate) mod agent_file_error;
pub(crate) mod agent_keys;
pub(crate) mod hook_command;
pub(crate) mod hook_file;
pub(crate) mod hook_file_error;
pub(crate) mod hook_file_part;
pub(crate) mod json_shape;
pub(crate) mod session_source;

pub use hook_file::HOOK_TIMEOUT_S;
