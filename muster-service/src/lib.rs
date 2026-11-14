//! The headless service side of muster: one service per user that supervises
//! every project's processes through holders, the events that drive it and
//! the notices it raises. The protocol clients reach it by lives in
//! `muster_protocol::service`.

mod event_loop;
mod events;
mod holder_client;
mod instance;
mod notice;
mod output;
mod readiness;
mod recording;
mod sampling;
mod service_error;
mod shells;
pub mod supervisor;
mod wait;

pub use event_loop::*;
pub use events::*;
pub use holder_client::*;
pub use instance::*;
pub use notice::*;
pub use output::*;
pub use readiness::*;
pub use recording::*;
pub use sampling::*;
pub use service_error::*;
pub use shells::*;
pub use wait::*;
