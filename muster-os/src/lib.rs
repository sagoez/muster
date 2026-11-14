//! The unix helpers muster's service, holders, clients and stores share:
//! private state and socket directories, process groups and signals, detached
//! spawns, bounded reads, and text made safe to print.

pub mod clock;
mod files;
mod process;
pub mod sanitize;
#[cfg(any(test, feature = "testing"))]
mod testing;

pub use files::{bounded_read, no_state_dir, path, private_dir};
pub use process::{child_group, open_files, process_identity, signal_hold, signal_target, spawn};
#[cfg(any(test, feature = "testing"))]
pub use testing::{test_fifo, test_scratch, test_wait};
