//! `muster mcp context`: the session context and how its shared text is
//! rendered.

pub(crate) mod rendering;
pub(crate) mod session_context;

pub use session_context::{CONTEXT_DEADLINE, CONTEXT_MAX_BYTES};
