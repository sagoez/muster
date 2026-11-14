//! A running pane's lifecycle: stops, exits, restarts and readiness.

pub(crate) mod lifecycle;
pub(crate) mod restart_book;
pub(crate) mod run_end;
pub(crate) mod run_readiness;
pub(crate) mod shutdown_generation;

pub use shutdown_generation::*;
