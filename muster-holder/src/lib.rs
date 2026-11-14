//! The holder: a small detached process that owns one child's PTY, terminal
//! emulator and log, and serves them to the service and attached terminals
//! over a framed unix socket, outliving the service that started it.

mod child;
mod clients;
#[cfg(any(test, feature = "testing"))]
mod fixture;
mod launcher;
pub mod pty;
mod serve;

#[cfg(any(test, feature = "testing"))]
pub use clients::test_client;
#[cfg(any(test, feature = "testing"))]
pub use fixture::{channel_sink, harness, holders, testing, thread_launcher};
pub use launcher::*;
pub use serve::*;
