//! The built `muster` binary end to end: the CLI as a user runs it, the
//! service as a client starts it, and a holder launched detached, each over
//! isolated state that every test cleans up.

#![cfg(target_os = "linux")]

mod cli;
mod common;
mod holder;
mod service;
