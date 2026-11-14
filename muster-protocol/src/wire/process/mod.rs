//! What a process carries over the wire: its spawn, stop policy, exits,
//! patterns and listening ports.

mod exit_cause_wire;
mod exit_outcome_wire;
mod listening_socket_wire;
mod log_pattern_wire;
mod port_occupant_wire;
mod pty_size_wire;
mod spawn_history_wire;
mod spawn_request_wire;
mod stop_policy_wire;

pub use exit_cause_wire::*;
pub use exit_outcome_wire::*;
pub use listening_socket_wire::*;
pub use log_pattern_wire::*;
pub use port_occupant_wire::*;
pub use pty_size_wire::*;
pub use spawn_history_wire::*;
pub use spawn_request_wire::*;
pub use stop_policy_wire::*;
