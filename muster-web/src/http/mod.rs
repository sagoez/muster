//! The view's own small HTTP server: GET only, one request per connection,
//! with a bounded head, a deadline and a connection cap, and the gate every
//! request passes.

mod access;
mod connection;
mod reply;

pub use access::*;
pub use connection::*;
pub use reply::*;
