//! Starting this user's service and reporting on it.

pub(crate) mod running_service;
#[cfg(any(test, feature = "testing"))]
pub mod served_service;
pub(crate) mod service_log;
pub(crate) mod service_start;
pub(crate) mod service_status;
pub(crate) mod this_binary;
pub(crate) mod unavailable_reason;
mod user_service;

pub use running_service::*;
pub use service_log::*;
pub use service_status::*;
pub use this_binary::*;
pub use unavailable_reason::*;
pub use user_service::*;
