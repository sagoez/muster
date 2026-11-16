//! Moving the state directory an earlier muster kept elsewhere on this
//! platform to where muster keeps it now, and saying what keeps it apart.

#[cfg(test)]
mod legacy_home;
mod legacy_state_move;
mod move_failure;
mod settling;
mod state_move_error;

pub use legacy_state_move::*;
pub use settling::*;
pub use state_move_error::*;
