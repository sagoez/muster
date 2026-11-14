//! One connection served: the acceptor, the connection cap, the bounded head
//! read and what answers the request.

mod connection_slot;
mod head_read;
pub(crate) mod request;
mod responder;
mod server;

pub use request::*;
pub use responder::*;
pub use server::*;
