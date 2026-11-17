//! `muster mcp context`: the session context, its two listings and how its
//! shared text is rendered.

pub(crate) mod listing;
pub(crate) mod rendering;
pub(crate) mod session_context;

pub use session_context::{CONTEXT_DEADLINE, CONTEXT_MAX_BYTES};
