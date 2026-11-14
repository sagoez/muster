//! `muster view`: what the CLI asks the composition root to serve, and what it reports back.

pub(crate) mod view_launch;
pub(crate) mod view_started;

pub use view_launch::*;
pub use view_started::*;
