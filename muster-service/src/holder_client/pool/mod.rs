//! Every holder the service can reach, shared by its supervisors.

#[cfg(test)]
pub(crate) mod fake_pool;
pub(crate) mod holder_pool;
#[cfg(test)]
pub(crate) mod null_sink;
pub(crate) mod shared_holders;
#[cfg(test)]
mod stalled_handle;

pub use holder_pool::*;
pub use shared_holders::*;
