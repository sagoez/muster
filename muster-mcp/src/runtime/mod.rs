//! The rmcp server and its runtime: what it works with, what it tells
//! agents, and how long a call may block.

pub(crate) mod bounds;
mod described;
pub(crate) mod mcp_deps;
mod mcp_error;
pub(crate) mod progress;
pub(crate) mod server;
mod stdio_runtime;
#[cfg(test)]
pub(crate) mod test_support;

pub use mcp_deps::*;
pub use mcp_error::*;
pub use server::{MusterMcp, connection_config};
pub use stdio_runtime::*;
