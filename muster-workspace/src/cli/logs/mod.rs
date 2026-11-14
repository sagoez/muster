//! `muster logs`: reading, following and filtering a process's output, made
//! safe to print.

mod file_identity;
pub(crate) mod followed_output;
pub(crate) mod json_log_lines;
mod kept_bytes;
pub(crate) mod log_filter_args;
pub(crate) mod log_follower;
pub(crate) mod project_log_slice;
pub(crate) mod rendered_log;
pub(crate) mod terminal_safe;

pub use json_log_lines::*;
pub use log_filter_args::*;
pub use log_follower::*;
