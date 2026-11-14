//! The view's own small HTTP server: GET only, one request per connection,
//! with a bounded head, a deadline and a connection cap, and the gate every
//! request passes.

mod admission;
mod cache_policy;
mod connection_slot;
mod gate;
mod head_read;
#[cfg(any(test, feature = "testing"))]
mod http_reply;
mod one_time_code;
mod open_code;
mod request;
mod responder;
mod response;
mod server;
mod status;
mod token;

pub use admission::*;
pub use cache_policy::*;
pub use gate::*;
#[cfg(any(test, feature = "testing"))]
pub use http_reply::*;
pub use request::*;
pub use responder::*;
pub use response::*;
pub use server::*;
pub use status::*;
pub use token::*;
