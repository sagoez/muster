//! Telling the service that stored state changed.

#[cfg(any(test, feature = "testing"))]
pub mod recording_notifier;
mod running_service_notifier;

pub use running_service_notifier::*;
