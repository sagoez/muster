use std::sync::{Arc, PoisonError, mpsc::SyncSender};

use typed_builder::TypedBuilder;

use super::{
    attach_event::AttachEvent, ending_signal::EndingSignal, error::CliError,
    session_alarm::SessionAlarm, signal_slot::SignalSlot, signal_source::SignalSource,
};

/// A signal source a test drives: it posts `preset` as soon as the
/// session listens, then whatever the test sends through `slot`, and
/// reports `late` as it is stopped, as signals arriving during teardown
/// would, the way the real listener does: recorded in the alarm, posted
/// only when there is room.
#[derive(TypedBuilder)]
pub(super) struct ScriptedSignals {
    /// The session's event sender and alarm, while it listens.
    slot: SignalSlot,
    /// Events posted the moment the session listens.
    preset: Vec<AttachEvent>,
    /// Ending signals arriving while the session stops listening.
    late: Vec<EndingSignal>,
}

impl SignalSource for ScriptedSignals {
    fn listen(
        &mut self,
        events: SyncSender<AttachEvent>,
        alarm: Arc<SessionAlarm>,
    ) -> Result<(), CliError> {
        for event in self.preset.drain(..) {
            events.send(event).unwrap();
        }
        *self.slot.lock().unwrap_or_else(PoisonError::into_inner) = Some((events, alarm));
        Ok(())
    }

    fn stop(&mut self) {
        let listening = self
            .slot
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some((events, alarm)) = listening {
            for signal in self.late.drain(..) {
                alarm.record_ending(signal);
                let _ = events.try_send(AttachEvent::Ended(signal));
            }
        }
    }
}
