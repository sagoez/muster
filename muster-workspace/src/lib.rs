//! muster - the supervisor and control plane behind your terminal: a per-user
//! service that runs each project's dev processes, with a CLI and an MCP
//! surface to drive them.
//!
//! The workspace is organized as a hexagon: a pure domain ([`muster_domain`]),
//! and adapters, in this crate and its siblings, that plug concrete
//! infrastructure into the domain's ports. The binary in `main.rs` is a thin
//! composition root over this library.

pub mod cli;
pub mod detect;
pub mod error;
pub mod exchange;
pub mod listening;
pub mod metrics;
pub mod notifier;
pub mod port_occupants;
pub mod process_tree;
pub mod shell;
