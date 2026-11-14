//! The MCP server an agent launches over stdio: each project's stack
//! definition, its live processes through the muster service, the machine's
//! settings, and the shared notes, todos, key-values and locks. Each call
//! resolves its own project.

mod caller;
mod config;
mod coordination;
mod open;
mod process;
mod result;
mod runtime;
mod stack;

pub use caller::*;
pub use result::*;
pub use runtime::*;
