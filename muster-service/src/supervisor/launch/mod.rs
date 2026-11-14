//! Starting a pane: launches in flight, their results, and the refusal of a
//! start whose port is already held.

mod blocking;
pub(crate) mod channel_output_sink;
pub(crate) mod launch_book;
pub(crate) mod launch_purpose;
pub(crate) mod pending_intent;
pub(crate) mod pending_launch;
pub(crate) mod shared_port_occupants;
pub(crate) mod spawn_generation;
pub(crate) mod stale_release;

pub use channel_output_sink::*;
pub use spawn_generation::*;
pub use stale_release::*;
