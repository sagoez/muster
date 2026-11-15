//! The pollers that feed the hub, each on a thread of its own and each
//! ending once the hub is gone: the projects, the service's status and its
//! events, the notes, and the logs' stats.

pub(crate) mod events_poll;
mod file_identity;
pub(crate) mod logs_poll;
pub(crate) mod notes_poll;
pub(crate) mod pollers;
pub(crate) mod projects_poll;
pub(crate) mod state_access;
pub(crate) mod status_poll;

pub use pollers::*;
