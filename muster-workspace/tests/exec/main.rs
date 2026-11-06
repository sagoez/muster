//! The built `muster` binary end to end: the CLI as a user runs it, the
//! service as a client starts it, a holder launched detached, bash, zsh
//! and fish running the shell hook on a pseudo-terminal, and `open` in a
//! real herdr when asked for, each over isolated state that every test
//! cleans up.

#![cfg(target_os = "linux")]

mod cli;
mod common;
mod herdr;
mod holder;
mod service;
mod shell_hook;
