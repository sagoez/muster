//! The binary driven as a person or an agent would: a PTY, an interactive
//! shell, an MCP session and a running view.

mod mcp_session;
mod pty;
pub(crate) mod shell_session;
mod view_session;

pub use mcp_session::*;
pub use pty::*;
pub use shell_session::*;
pub use view_session::*;
