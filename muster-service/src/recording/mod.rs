//! How runs ended and which edits were raised, written to the store off the
//! loop.

#[cfg(test)]
mod failing_history;
pub(crate) mod record_retry;
pub(crate) mod recorder_stores;
mod store_record;
pub(crate) mod store_recorder;

pub use record_retry::*;
pub use store_recorder::*;
