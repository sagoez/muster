//! How a command presents itself: frames, spinners, the line it ends with,
//! and what it writes to stdout.

pub(crate) mod emit;
mod interrupt_flags;
mod interrupt_guard;
pub(crate) mod next_step;
pub(crate) mod presenter;
mod presenter_theme;
mod spinners;
mod surroundings;

pub use presenter::*;
pub use spinners::*;
pub use surroundings::*;
