//! muster's domain: processes, stacks, settings, coordination, and the ports
//! its adapters implement. Pure: no I/O, no framework types, no `async`.

pub mod color_scheme;
pub mod config;
pub mod constants;
pub mod coordination;
pub mod notification;
pub mod occupancy;
pub mod port;
pub mod process;
pub mod project;
pub mod pty;
pub mod settings;
pub mod terminal_workspace;
pub mod value;
