use std::{
    fs::File,
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex, PoisonError, mpsc::SyncSender},
    thread::{self, JoinHandle},
};

use getset::{Getters, MutGetters};
use muster_os::test_wait::wait_until;
use muster_protocol::service::Located;

use crate::cli::{
    failure::error::CliError,
    session::{
        attach::AttachSession,
        attach_testing::{detach, name},
        host::{host_terminal::HostTerminal, test_pty::TestPty},
        pump::{announce::Announce, attach_end::AttachEnd, attach_event::AttachEvent},
        signals::{
            ending_signal::EndingSignal, scripted_signals::ScriptedSignals,
            session_alarm::SessionAlarm, signal_slot::SignalSlot,
        },
    },
};

/// A session running on a thread, with a way to send it events a signal
/// would.
#[derive(Getters, MutGetters)]
pub(crate) struct Running {
    /// The session's thread.
    #[getset(get = "pub(crate)", get_mut = "pub(crate)")]
    session: JoinHandle<Result<AttachEnd, CliError>>,
    /// Where signal events go.
    signals: SignalSlot,
}

impl Running {
    /// Attaches to `socket` from `pty`.
    pub(crate) fn start(pty: &TestPty, socket: PathBuf, read_only: bool) -> Self {
        Self::on(pty.terminal(), socket, read_only, Vec::new(), Vec::new())
    }

    /// Attaches to `socket` from `terminal`, with `preset` signal events
    /// already waiting and `late` ending signals arriving as signals stop.
    pub(crate) fn on(
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
    pub(crate) fn signal(&self, event: AttachEvent) {
        self.listening().0.send(event).unwrap();
    }

    /// The session's alarm, once it listens.
    pub(crate) fn alarm(&self) -> Arc<SessionAlarm> {
        self.listening().1
    }

    /// Waits for the session to end, answering how.
    pub(crate) fn end(self) -> Result<AttachEnd, CliError> {
        wait_until("the session to end", || self.session.is_finished());
        self.session.join().unwrap()
    }
}
