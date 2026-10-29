use std::io;

use thiserror::Error;

/// A failure starting or running the coordination MCP server over stdio.
#[derive(Debug, Error)]
pub enum McpError {
    /// The async runtime the server runs on could not be built.
    #[error("could not start the MCP runtime: {0}")]
    Runtime(#[source] io::Error),
    /// The server could not be initialized on the transport.
    #[error("could not start the MCP server: {0}")]
    Serve(String),
    /// The server stopped with an error rather than a clean shutdown.
    #[error("the MCP server stopped abnormally: {0}")]
    Wait(String),
}
