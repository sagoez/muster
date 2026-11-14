//! Reading a process's log off the loop for a wait held on it.

mod log_answer;
pub(crate) mod log_ask;
mod log_read_job;
mod log_read_result;
mod log_reads;
pub(crate) mod pending_read;

pub use log_answer::*;
pub use log_read_result::*;
