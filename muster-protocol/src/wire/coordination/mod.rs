//! Coordination entries as a snapshot keeps them and an agent reads them.

mod author_wire;
mod epoch_seconds_wire;
mod key_value_entry;
mod scratchpad_entry;
mod todo_claim_entry;
mod todo_entry;

pub use key_value_entry::*;
pub use scratchpad_entry::*;
pub use todo_claim_entry::*;
pub use todo_entry::*;
