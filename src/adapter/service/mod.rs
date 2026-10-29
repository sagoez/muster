//! The headless service side of muster: one service per user that supervises
//! every project's processes through holders, the protocol clients reach it
//! by, the events that drive it and the notices it raises.

mod acceptor;
mod batch_outcome;
mod event;
mod holder_pool;
mod launch_environment;
mod located;
mod notice;
mod notice_level;
mod notices;
mod printable;
mod process_action;
mod process_status;
mod process_usage;
mod project_ports;
mod project_status;
pub mod protocol;
mod refusal;
mod runtime;
mod service_deps;
mod service_error;
mod service_error_body;
mod service_error_kind;
mod service_request;
mod service_response;
mod shared_holders;
mod singleton;
mod state_dir;
pub mod supervisor;

pub use batch_outcome::*;
pub use event::*;
pub use holder_pool::*;
pub use launch_environment::*;
pub use located::*;
pub use notice::*;
pub use notice_level::*;
pub use notices::*;
pub use printable::*;
pub use process_action::*;
pub use process_status::*;
pub use process_usage::*;
pub use project_ports::*;
pub use project_status::*;
pub use refusal::*;
pub use runtime::*;
pub use service_deps::*;
pub use service_error::*;
pub use service_error_body::*;
pub use service_error_kind::*;
pub use service_request::*;
pub use service_response::*;
pub use shared_holders::*;
pub use singleton::*;
pub use state_dir::*;
