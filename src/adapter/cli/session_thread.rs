use std::{
    panic::{self, AssertUnwindSafe},
    sync::{Arc, mpsc::SyncSender},
    thread::{self, JoinHandle},
};

use super::{
    attach_event::AttachEvent,
    session_alarm::{SessionAlarm, mark_session_helper},
};

/// Runs `body` on a helper thread of the attach session `alarm` belongs to,
/// posting to `events`. A panic in it ends the session: it is recorded in
/// `alarm`, which the session checks however full its inbox is, and posted
/// as [`AttachEvent::HelperFailed`] when there is room, never waiting for it.
pub fn spawn_session_thread(
    events: SyncSender<AttachEvent>,
    alarm: Arc<SessionAlarm>,
    body: impl FnOnce(&SyncSender<AttachEvent>) + Send + 'static,
) -> JoinHandle<()> {
    thread::spawn(move || {
        mark_session_helper(Arc::clone(&alarm));
        if panic::catch_unwind(AssertUnwindSafe(|| body(&events))).is_err() {
            alarm.record_helper_failure();
            let _ = events.try_send(AttachEvent::HelperFailed);
        }
    })
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::sync_channel;

    use super::*;

    /// A helper that panics tells its session so, through its alarm even
    /// when the inbox is full; one that returns does not.
    #[test]
    fn a_panicking_helper_ends_its_session() {
        let (events, inbox) = sync_channel(1);
        let alarm = Arc::new(SessionAlarm::default());
        spawn_session_thread(events, Arc::clone(&alarm), |_| panic!("a bug in a reader"))
            .join()
            .unwrap();
        assert!(matches!(inbox.try_recv(), Ok(AttachEvent::HelperFailed)));
        assert!(alarm.helper_failed());

        let (events, _inbox) = sync_channel(1);
        events.send(AttachEvent::Resized).unwrap();
        let alarm = Arc::new(SessionAlarm::default());
        spawn_session_thread(events, Arc::clone(&alarm), |_| panic!("a bug in a reader"))
            .join()
            .unwrap();
        assert!(
            alarm.helper_failed(),
            "a full inbox does not lose the failure"
        );

        let (events, inbox) = sync_channel(1);
        let alarm = Arc::new(SessionAlarm::default());
        spawn_session_thread(events, Arc::clone(&alarm), |_| {})
            .join()
            .unwrap();
        assert!(inbox.try_recv().is_err());
        assert!(!alarm.helper_failed());
    }
}
