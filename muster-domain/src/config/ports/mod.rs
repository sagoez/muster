//! The ports a process declares, and why a set of them is refused.

pub(crate) mod declared_ports;
mod repeated_port;
mod too_many_ports;

pub use declared_ports::*;
pub use repeated_port::*;
pub use too_many_ports::*;
