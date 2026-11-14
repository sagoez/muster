//! The SQLite store behind every store port: projects, stacks, settings,
//! coordination, run history and the edit log, and the migrations that move
//! the database forward.

mod coordination;
mod history;
mod sqlite_store;
mod state;
mod store_error;
#[cfg(any(test, feature = "testing"))]
pub mod test_support;

pub use sqlite_store::*;
pub use store_error::*;
