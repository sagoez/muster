use std::{
    fs,
    thread::{self, JoinHandle},
    time::Duration,
};

use getset::{Getters, MutGetters};
use muster_domain::{color_scheme::ColorScheme, pty::PtySize};
use muster_os::{signal_target::SignalTarget, test_wait::wait_until};
use muster_protocol::holder::{
    ClientMessage, ClientRole, HolderConfig, HolderControl, HolderError, HolderId, HolderMessage,
    HolderRecord,
};
use rustix::process::Signal;
use tempfile::TempDir;

use crate::{
    clients::test_client::TestClient,
    pty::PortablePtyRunner,
    serve::{
        server::HolderServer,
        server_testing::{SOCKET_DIR, SOCKET_FILE, SUPERVISOR_DEADLINE, request, scratch},
    },
};

/// A running holder on a thread, with its scratch directory.
#[derive(Getters, MutGetters)]
pub struct Harness {
    /// The holder's configuration.
    #[getset(get = "pub", get_mut = "pub")]
    config: HolderConfig,
    /// The thread serving the holder, until it is joined.
    #[getset(get = "pub", get_mut = "pub")]
    server: Option<JoinHandle<Result<(), HolderError>>>,
    /// Holds the scratch directory for the harness's life.
    _scratch: TempDir,
}

impl Harness {
    /// A holder running `command` in a fresh scratch directory.
    pub fn start(command: &str) -> Self {
        Self::start_with(command, SUPERVISOR_DEADLINE, ColorScheme::Dark)
    }

    /// As [`Self::start`], killing its child unless a supervisor reaches it by
    /// `supervisor_deadline`.
    pub fn start_with_deadline(command: &str, supervisor_deadline: Duration) -> Self {
        Self::start_with(command, supervisor_deadline, ColorScheme::Dark)
    }

    /// A holder running `command` under `color_scheme`, with
    /// `supervisor_deadline`, once its record is written.
    pub fn start_with(
        command: &str,
        supervisor_deadline: Duration,
        color_scheme: ColorScheme,
    ) -> Self {
        let scratch = scratch();
        let dir = scratch.path();
        let config = HolderConfig::builder()
            .id(HolderId::generate().unwrap())
            .request(request(dir, command, color_scheme))
            .socket(dir.join(SOCKET_DIR).join(SOCKET_FILE))
            .record_dir(dir.join("records"))
            .log(dir.join("logs").join("dev.log"))
            .supervisor_deadline(supervisor_deadline)
            .build();
        let server_config = config.clone();
        let server = thread::spawn(move || HolderServer::run(server_config, &PortablePtyRunner));
        let harness = Self {
            config,
            server: Some(server),
            _scratch: scratch,
        };
        wait_until("the holder record", || harness.record().is_some());
        harness
    }

    /// The holder's record, while it is written.
    pub fn record(&self) -> Option<HolderRecord> {
        HolderRecord::load_all(self.config.record_dir()).pop()
    }

    /// The child's log so far, empty while it cannot be read.
    pub fn log(&self) -> String {
        String::from_utf8_lossy(&fs::read(self.config.log()).unwrap_or_default()).into_owned()
    }

    /// A client connected as `role`, with the snapshot it was greeted with.
    pub fn connect(&self, role: ClientRole) -> (TestClient, HolderMessage) {
        TestClient::connect(self.config.socket(), role)
    }

    /// Whether the holder's thread has returned.
    pub fn server_finished(&self) -> bool {
        self.server.as_ref().is_none_or(JoinHandle::is_finished)
    }

    /// The PTY size a fresh read-only client is told.
    pub fn current_size(&self) -> PtySize {
        match self.connect(ClientRole::Attach { read_only: true }).1 {
            HolderMessage::Snapshot { size, .. } => size,
            other => panic!("expected a snapshot, got {other:?}"),
        }
    }

    /// Kills the child, collects its exit and checks the holder cleaned up.
    pub fn finish(self) {
        let (mut supervisor, _) = self.connect(ClientRole::Supervisor);
        supervisor.send(ClientMessage::Control(HolderControl::Kill));
        supervisor.recv_until(|message| matches!(message, HolderMessage::Exited(_)));
        supervisor.send(ClientMessage::CollectExit);
        self.finish_collected();
    }

    /// Joins a holder whose exit a test already collected.
    pub fn finish_collected(mut self) {
        wait_until("the holder to return", || self.server_finished());
        let server = self.server.take().unwrap();
        server.join().unwrap().unwrap();
        assert!(self.record().is_none(), "the record is removed");
        assert!(!self.config.socket().exists(), "the socket is removed");
    }
}

impl Drop for Harness {
    /// Kills the process group of a child a failed test left running, so a
    /// panicking test does not leak it; the scratch directory drops after.
    fn drop(&mut self) {
        if !self.server_finished()
            && let Some(record) = self.record()
        {
            let _ = SignalTarget::Group(record.child_pid()).send(Signal::KILL);
        }
    }
}
