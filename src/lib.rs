//! muster - a ratatui terminal workspace for running CLI agents and dev
//! processes side by side.
//!
//! The crate is organized as a hexagon: a pure [`domain`], and [`adapter`]s that
//! plug concrete infrastructure into the domain's ports. The binary in
//! `main.rs` is a thin composition root over this library.

pub mod adapter;
pub mod application;
pub mod constants;
pub mod domain;
pub mod error;
