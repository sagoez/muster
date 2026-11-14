//! The service's one event loop: the runtime, its events and deps, and the
//! fixtures its tests share.

pub(crate) mod event;
mod flow;
#[cfg(test)]
mod memory_store;
pub(crate) mod project_ports;
mod reply;
mod runtime;
#[cfg(test)]
mod runtime_fixture;
#[cfg(test)]
pub(crate) mod runtime_testing;
mod service_deps;
#[cfg(test)]
mod test_ports;

pub use event::*;
pub use project_ports::*;
pub use runtime::*;
pub use service_deps::*;
