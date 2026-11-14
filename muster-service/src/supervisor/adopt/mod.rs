//! Adopting the holders of children left running, retrying those that fail,
//! and the children no holder can be reached for.

pub(crate) mod adopter;
mod adoption;
pub(crate) mod adoption_retries;
pub(crate) mod carried_pane;
mod pane_retry;
mod retry_id;
mod unsupervised;
pub(crate) mod unsupervised_child;

pub use adopter::*;
pub use retry_id::*;
