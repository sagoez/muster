//! `ClientError`, and how a failure reports to a script or an agent.

pub(crate) mod client_error;
mod failure_kind;
mod failure_view;

pub use client_error::*;
pub use failure_kind::*;
pub use failure_view::*;
