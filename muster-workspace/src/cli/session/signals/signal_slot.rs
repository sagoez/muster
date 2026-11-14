use std::sync::{Arc, Mutex, mpsc::SyncSender};

use crate::cli::session::{pump::attach_event::AttachEvent, signals::session_alarm::SessionAlarm};

/// Where a test sends the events a signal would, and the session's alarm,
/// while the session listens.
pub(crate) type SignalSlot = Arc<Mutex<Option<(SyncSender<AttachEvent>, Arc<SessionAlarm>)>>>;
