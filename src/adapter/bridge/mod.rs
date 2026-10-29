//! The transport contract between an MCP client and the service for what the
//! service's other requests do not cover. It lives in the adapter layer, so
//! the core never sees it.

mod workspace_request;

pub use workspace_request::*;
