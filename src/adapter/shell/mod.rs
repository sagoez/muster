//! The shells running muster's hook: the hook each one loads, and the files
//! each one's reports and notes are kept in.

mod hook_script;
mod hook_script_error;
mod hook_shell;
mod shell_files;

pub use hook_script::*;
pub use hook_script_error::*;
pub use hook_shell::*;
pub use shell_files::*;
