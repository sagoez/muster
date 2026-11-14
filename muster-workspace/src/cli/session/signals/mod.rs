//! The signals that end or resize a session, and the alarm they ring.

pub(crate) mod ending_signal;
pub(crate) mod host_signals;
#[cfg(test)]
pub(crate) mod scripted_signals;
pub(crate) mod session_alarm;
#[cfg(test)]
pub(crate) mod signal_slot;
pub(crate) mod signal_source;

pub use ending_signal::*;
pub use host_signals::*;
pub use session_alarm::{SessionAlarm, hold_helper_panic};
pub use signal_source::*;
