//! The clients connected to a holder.

pub(crate) mod client_id;
pub(crate) mod connected_client;
#[cfg(any(test, feature = "testing"))]
pub mod test_client;
