use std::sync::{Arc, Mutex, mpsc::SyncSender};

use super::{attach_event::AttachEvent, session_alarm::SessionAlarm};

/// Where a test sends the events a signal would, and the session's alarm,
/// while the session listens.
pub(super) type SignalSlot = Arc<Mutex<Option<(SyncSender<AttachEvent>, Arc<SessionAlarm>)>>>;
