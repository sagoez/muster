//! The session loop, its events and ending, and the holder it talks to.

pub(crate) mod announce;
pub(crate) mod attach_end;
pub(crate) mod attach_event;
pub(crate) mod attach_pump;
#[cfg(test)]
pub(crate) mod fake_holder;
#[cfg(test)]
pub(crate) mod running_session;
#[cfg(test)]
pub(crate) mod served_holder;
pub(crate) mod session_thread;

pub use announce::*;
pub use attach_end::*;
pub use attach_event::*;
