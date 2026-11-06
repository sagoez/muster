//! What muster's service, holders and clients exchange: the DTOs domain
//! values travel as, framed IPC over Unix sockets, and the requests the MCP
//! server sends the service.

pub mod holder;
pub mod ipc;
#[cfg(test)]
mod ping;
#[cfg(test)]
mod pong;
pub mod service;
pub mod wire;
pub mod workspace_request;
