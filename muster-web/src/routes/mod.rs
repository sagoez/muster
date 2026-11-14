//! What the view serves behind its gate, and the paths that name it.

mod asset;
mod event_stream;
mod log_answer;
mod log_tail;
mod log_target;
mod route;
mod shell;
mod site;
#[cfg(test)]
pub(crate) mod testing;

pub use site::*;
