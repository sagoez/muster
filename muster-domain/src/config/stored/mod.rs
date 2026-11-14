//! What a stored row the domain refuses says, such as one past a cap.

pub(crate) mod over_cap;
mod stored_problem;
mod unreadable_project;

pub use over_cap::*;
pub use stored_problem::*;
pub use unreadable_project::*;
