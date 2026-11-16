//! `muster mcp context`: the session context, the hook input it reads and
//! the marks that print it once per session start.

pub(crate) mod hook_input;
pub(crate) mod session_context;
pub(crate) mod session_marks;

pub use session_context::{CONTEXT_DEADLINE, CONTEXT_MAX_BYTES};
