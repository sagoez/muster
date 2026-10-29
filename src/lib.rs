//! muster - the supervisor and control plane behind your terminal: a per-user
//! service that runs a project's dev processes and CLI agents, with a CLI and
//! an MCP surface to drive them.
//!
//! The crate is organized as a hexagon: a pure [`domain`], and [`adapter`]s that
//! plug concrete infrastructure into the domain's ports. The binary in
//! `main.rs` is a thin composition root over this library.

pub mod adapter;
pub mod application;
pub mod constants;
pub mod domain;
pub mod error;
