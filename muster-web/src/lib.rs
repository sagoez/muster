//! `muster view`: a live, read-only page of every project's processes,
//! served on loopback to a browser that carries the view's token. It never
//! starts the service or loads a project into it.

mod floor;
mod http;
mod icon;
mod manifest_reader;
mod routes;
mod view_deps;
mod view_server;
mod web_error;

pub use floor::*;
#[cfg(feature = "testing")]
pub use http::{HttpReply, exchange, get};
pub use icon::*;
pub use manifest_reader::*;
pub use view_deps::*;
pub use view_server::*;
pub use web_error::*;
