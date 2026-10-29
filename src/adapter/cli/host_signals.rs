use std::{
    ffi::c_int,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{SyncSender, TrySendError},
    },
    thread::JoinHandle,
};

use signal_hook::{
    consts::{SIGHUP, SIGTERM, SIGWINCH},
    flag::register_conditional_default,
    iterator::{Handle, Signals},
};

use super::{
    attach_event::AttachEvent, ending_signal::EndingSignal, error::CliError,
    session_alarm::SessionAlarm, session_thread::spawn_session_thread, signal_source::SignalSource,
};

/// The process's own signals: `SIGWINCH` as a resize, `SIGTERM` and `SIGHUP`
/// as the session's end.
#[derive(Default)]
pub struct HostSignals {
    /// The listener and its thread, while listening.
    listening: Option<(Handle, JoinHandle<()>)>,
    /// Set once listening stops, so an ending signal takes its default
    /// effect again: catching it would otherwise swallow it for good.
    ended: Arc<AtomicBool>,
}

impl SignalSource for HostSignals {
    fn listen(
        &mut self,
        events: SyncSender<AttachEvent>,
        alarm: Arc<SessionAlarm>,
    ) -> Result<(), CliError> {
        for signal in [SIGTERM, SIGHUP] {
            register_conditional_default(signal, Arc::clone(&self.ended))
                .map_err(CliError::Terminal)?;
        }
        let mut signals = Signals::new([SIGWINCH, SIGTERM, SIGHUP]).map_err(CliError::Terminal)?;
        let handle = signals.handle();
        let thread = spawn_session_thread(events, Arc::clone(&alarm), move |events| {
            relay(signals.forever(), events, &alarm);
        });
        self.listening = Some((handle, thread));
        Ok(())
    }

    fn stop(&mut self) {
        self.ended.store(true, Ordering::SeqCst);
        if let Some((handle, thread)) = self.listening.take() {
            handle.close();
            let _ = thread.join();
        }
    }
}

/// Passes each of `signals` to the session, never waiting on `events`, so
/// closing the listener always ends it: a resize finding the inbox full is
/// dropped, since resizes coalesce and the session may be ending, and an
/// ending signal is recorded in `alarm` before it is posted, so it is never
/// lost.
fn relay(
    signals: impl IntoIterator<Item = c_int>,
    events: &SyncSender<AttachEvent>,
    alarm: &SessionAlarm,
) {
    for signal in signals {
        let event = match EndingSignal::from_number(signal) {
            Some(ending) => {
                alarm.record_ending(ending);
                AttachEvent::Ended(ending)
            },
            None => AttachEvent::Resized,
        };
        if let Err(TrySendError::Disconnected(_)) = events.try_send(event) {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        sync::mpsc::{channel, sync_channel},
        thread,
    };

    use super::*;
    use crate::adapter::cli::test_pty::wait_until;

    /// A full inbox nothing drains neither blocks the relay nor loses the
    /// ending signal: the relay ends once its signals do, and the ending is
    /// in the alarm.
    #[test]
    fn a_full_inbox_never_blocks_the_relay() {
        let (events, _inbox) = sync_channel(1);
        events.send(AttachEvent::Resized).unwrap();
        let alarm = Arc::new(SessionAlarm::default());
        let (signal, signals) = channel();
        let relayed = Arc::clone(&alarm);
        let relaying = thread::spawn(move || relay(signals, &events, &relayed));

        signal.send(SIGWINCH).unwrap();
        signal.send(SIGTERM).unwrap();
        drop(signal);

        wait_until("the relay to end", || relaying.is_finished());
        relaying.join().unwrap();
        assert_eq!(alarm.ending(), Some(EndingSignal::Terminate));
    }
}
