//! A process's raw output rendered as the lines a log reader shows.

pub mod coloring;
#[cfg(test)]
pub mod fuzz_stream;
pub mod log_line;
pub mod log_renderer;
pub mod placed_renderer;
#[cfg(any(test, feature = "testing"))]
pub mod render_fault;
