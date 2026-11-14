//! Opening a running process in a pane beside the caller.

pub mod open;
mod open_request;
mod open_view;
#[cfg(any(test, feature = "testing"))]
pub mod recording_workspace;

pub use open_request::*;
pub use open_view::*;
