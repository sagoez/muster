//! The log route: the process a request names, the tail read off its log,
//! and the JSON that answers it.

mod log_answer;
mod log_error;
mod log_tail;
mod log_target;

pub use log_answer::*;
pub use log_error::*;
pub use log_tail::*;
pub use log_target::*;
