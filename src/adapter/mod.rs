pub mod bridge;
pub mod cli;
pub mod clipboard;
pub mod config;
pub mod editor;
pub mod hooks;
#[cfg(unix)]
pub mod ipc;
pub mod mcp;
pub mod metrics;
pub mod notifier;
pub mod path;
pub mod process_identity;
pub mod pty;
pub mod tui;
