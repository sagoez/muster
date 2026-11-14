use std::sync::{Arc, Mutex};

use super::event_log::EventLog;

/// The event log shared between the supervisors that record events and the
/// loop that answers reads of it.
pub type SharedEventLog = Arc<Mutex<EventLog>>;
