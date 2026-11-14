//! Waits: what a client waits for and how the wait ended.

mod matched_line;
mod wait_condition;
pub(crate) mod wait_outcome;
pub(crate) mod wait_spec;
pub(crate) mod wait_status;

pub use matched_line::*;
pub use wait_condition::*;
pub use wait_outcome::*;
pub use wait_spec::*;
pub use wait_status::*;
