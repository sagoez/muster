//! The processes a suite runs or looks for: holders it launches, the
//! processes running under a scratch, and the quiet ports a service run in
//! the test process works on.

mod held_launcher;
pub(crate) mod processes;
mod quiet_ports;

pub use held_launcher::*;
pub use processes::*;
pub use quiet_ports::*;
