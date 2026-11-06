#[cfg(test)]
pub(crate) mod channel_sink;
pub mod cli;
pub mod client;
pub mod detect;
pub mod exchange;
pub mod herdr;
pub mod holder;
pub mod ipc;
pub mod listening;
pub mod mcp;
pub mod metrics;
pub mod notifier;
#[cfg(test)]
pub(crate) mod ping;
#[cfg(test)]
pub(crate) mod pong;
pub mod port_occupants;
pub mod probe;
pub mod process_tree;
pub mod pty;
pub mod service;
pub mod shell;
pub mod store;
pub mod terminal;
pub mod wire;
pub mod workspace_request;
