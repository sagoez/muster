use thiserror::Error;

/// A failure starting or running the coordination MCP server over stdio.
#[derive(Debug, Error)]
pub enum McpError {
    /// The server could not be initialized on the transport.
    #[error("could not start the MCP server: {0}")]
    Serve(String),
    /// The server stopped with an error rather than a clean shutdown.
    #[error("the MCP server stopped abnormally: {0}")]
    Wait(String),
}
