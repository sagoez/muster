//! Headless supervision of one project's processes: spawning, stopping,
//! restarting and adopting them, independent of any terminal UI. The
//! [`Supervisor`] aggregate is declared in `entity` and its behaviour is split
//! across the modules of its feature folders, which share its state. The state
//! that is not about one pane's lifecycle lives in owned books (`restart_book`,
//! `launch_book`, `attention`, `metrics_book`, `attach_book`) that keep their
//! fields private.

mod adopt;
mod attention;
mod deps;
mod entity;
mod launch;
mod log_read;
mod pane;
mod run;
mod screen;
mod stack;
#[cfg(test)]
pub(crate) mod testing;
mod usage;
mod wait;

pub use adopt::*;
pub use deps::*;
pub use entity::*;
pub use launch::*;
pub use log_read::*;
pub use run::*;
pub use screen::*;
pub use stack::*;
pub use wait::*;
