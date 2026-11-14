//! `muster projects`: the registered projects and their idle stops.

pub(crate) mod idle_stop_override;
mod idle_stop_source;
mod idle_stop_view;
mod project_removal;
pub(crate) mod projects;
pub(crate) mod projects_command;

pub use idle_stop_override::*;
pub use projects_command::*;
