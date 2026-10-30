use std::{
    fs::File,
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError, mpsc::SyncSender},
    thread::{self, JoinHandle},
};

use getset::{Getters, MutGetters};

use super::{
    announce::Announce,
    attach::AttachSession,
    attach_end::AttachEnd,
    attach_event::AttachEvent,
    attach_testing::{detach, name},
    ending_signal::EndingSignal,
    error::CliError,
    host_terminal::HostTerminal,
    scripted_signals::ScriptedSignals,
    session_alarm::SessionAlarm,
    signal_slot::SignalSlot,
    test_pty::TestPty,
};
use crate::adapter::{service::Located, test_wait::wait_until};

/// A session running on a thread, with a way to send it events a signal
/// would.
#[derive(Getters, MutGetters)]
pub(super) struct Running {
    /// The session's thread.
    #[getset(get = "pub(super)", get_mut = "pub(super)")]
    session: JoinHandle<Result<AttachEnd, CliError>>,
    /// Where signal events go.
    signals: SignalSlot,
}

impl Running {
    /// Attaches to `socket` from `pty`.
    pub(super) fn start(pty: &TestPty, socket: PathBuf, read_only: bool) -> Self {
        Self::on(pty.terminal(), socket, read_only, Vec::new(), Vec::new())
    }

    /// Attaches to `socket` from `terminal`, with `preset` signal events
    /// already waiting and `late` ending signals arriving as signals stop.
    pub(super) fn on(
        terminal: HostTerminal,
        socket: PathBuf,
        read_only: bool,
        preset: Vec<AttachEvent>,
        late: Vec<EndingSignal>,
    ) -> Self {
        let signals: SignalSlot = Arc::new(Mutex::new(None));
        let source = ScriptedSignals::builder()
            .slot(Arc::clone(&signals))
            .preset(preset)
            .late(late)
            .build();
        let output = terminal.output().try_clone().unwrap();
        let announce: Announce = Box::new(move |line: &str| writeln!(File::from(output), "{line}"));
        let session = AttachSession::new(name(), terminal, Box::new(source), announce);
        let target = Located::builder()
            .socket(socket)
            .log(PathBuf::from("unused.log"))
            .build();
        let session = thread::spawn(move || session.run(target, read_only, detach()));
        Self { session, signals }
    }

    /// The session's event sender and alarm, once it listens.
    pub(super) fn listening(&self) -> (SyncSender<AttachEvent>, Arc<SessionAlarm>) {
        let mut listening = None;
        wait_until("the session to listen", || {
            listening = self
                .signals
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            listening.is_some()
        });
        listening.unwrap()
    }

    /// Sends `event` as a signal would, once the session listens.
    pub(super) fn signal(&self, event: AttachEvent) {
        self.listening().0.send(event).unwrap();
    }

    /// The session's alarm, once it listens.
    pub(super) fn alarm(&self) -> Arc<SessionAlarm> {
        self.listening().1
    }

    /// Waits for the session to end, answering how.
    pub(super) fn end(self) -> Result<AttachEnd, CliError> {
        wait_until("the session to end", || self.session.is_finished());
        self.session.join().unwrap()
    }
}
