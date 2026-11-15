//! What the view serves behind its gate, and the paths that name it.

mod asset;
mod event_stream;
mod log;
mod route;
mod shell;
mod site;
#[cfg(test)]
pub(crate) mod testing;

pub(crate) use event_stream::frame;
pub use site::*;
