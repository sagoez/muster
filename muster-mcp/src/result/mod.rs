//! What every tool answers, and how it refuses.

mod agent_hint;
pub(crate) mod alerted;
pub(crate) mod introduced;
pub(crate) mod registered;
pub(crate) mod tool_error;
pub(crate) mod tool_failure;
pub(crate) mod tool_result;

pub use tool_error::*;
pub use tool_failure::*;
