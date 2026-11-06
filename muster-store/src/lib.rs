//! The SQLite store behind every store port: projects, stacks, settings,
//! coordination, run history and the edit log, and the migrations that move
//! the database forward.

mod coordination_rows;
mod edit_log_rows;
mod idle_stop_kind;
mod idle_stop_refusal;
mod lock_rows;
mod pending_idle_stop_rows;
mod project_rows;
mod refusal;
mod run_rows;
mod settings_rows;
mod sqlite_store;
mod stack_rows;
mod store_error;
#[cfg(any(test, feature = "testing"))]
pub mod test_support;

pub use sqlite_store::*;
pub use store_error::*;
