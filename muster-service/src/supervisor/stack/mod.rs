//! The project as a whole: applying its stack, finding a process by name,
//! its idle stop and its status snapshot.

mod apply_stack;
pub(crate) mod config_membership;
mod idle;
pub(crate) mod lookup;
mod snapshot;

pub use config_membership::*;
