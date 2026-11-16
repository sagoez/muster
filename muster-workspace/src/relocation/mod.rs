//! Moving the state directory an earlier muster kept elsewhere on this
//! platform to where muster keeps it now, and holding the state directory a
//! service runs from while it may still move.

#[cfg(test)]
mod legacy_home;
mod legacy_state_move;
mod state_dir_hold;
mod state_move_error;

pub use legacy_state_move::*;
pub use state_dir_hold::*;
pub use state_move_error::*;
