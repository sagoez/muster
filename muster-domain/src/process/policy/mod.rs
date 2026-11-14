//! How a process is restarted and stopped, and what its exit leads to.

mod exit_intent;
mod restart;
mod stop;
mod stop_signal;

pub use exit_intent::*;
pub use restart::*;
pub use stop::*;
pub use stop_signal::*;
