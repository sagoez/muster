//! A herdr stand-in: the fake binary a suite puts on its path, the
//! environment that names a pane, and the server that answers as one.

mod fake_herdr;
pub(crate) mod herdr_env;
mod herdr_server;

pub use fake_herdr::*;
pub use herdr_env::HERDR_PANE_ENV;
pub use herdr_server::*;
