//! Headless supervision of one project's processes: spawning, stopping,
//! restarting and adopting them, independent of any terminal UI. The
//! [`Supervisor`] aggregate is declared in `entity` and its behaviour is split
//! across the sibling modules, which share its state.

mod adopter;
mod apply_stack;
mod bridge;
mod carried_pane;
mod channel_output_sink;
mod config_membership;
mod deps;
mod entity;
mod holder_screen;
mod launch_purpose;
mod lifecycle;
mod metrics;
mod pane;
mod pending_intent;
mod pending_launch;
mod run_end;
mod run_readiness;
mod screen_reply;
mod shutdown_generation;
mod spawn_generation;
mod stale_release;
#[cfg(test)]
pub(crate) mod testing;
mod unsupervised;
mod unsupervised_child;
mod waiting;

pub use adopter::*;
pub use channel_output_sink::*;
pub use config_membership::*;
pub use deps::*;
pub use entity::*;
pub use holder_screen::*;
pub use screen_reply::*;
pub use shutdown_generation::*;
pub use spawn_generation::*;
pub use stale_release::*;
pub use waiting::*;
