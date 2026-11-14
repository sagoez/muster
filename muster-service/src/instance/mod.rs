//! The one service per user: its lock, its state directory and its socket.

pub(crate) mod acceptor;
#[cfg(test)]
pub(crate) mod running;
pub(crate) mod singleton;
pub(crate) mod state_dir;

pub use singleton::*;
pub use state_dir::*;
