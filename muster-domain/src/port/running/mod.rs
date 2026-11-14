//! Ports for spawning a process and driving it: input, output and screen.

mod output_sink;
mod process_handle;
mod process_input;
mod process_runner;
mod screen_reader;

pub use output_sink::*;
pub use process_handle::*;
pub use process_input::*;
pub use process_runner::*;
pub use screen_reader::*;
