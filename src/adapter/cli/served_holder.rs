use std::{
    os::unix::net::UnixStream,
    path::PathBuf,
    thread::{self, JoinHandle},
    time::Duration,
};

use bytesize::ByteSize as RawByteSize;
use rustix::process::Signal;
use tempfile::TempDir;

use super::{
    attach_testing::{COLS, ROWS, SCROLLBACK, SUPERVISOR_DEADLINE, name, scratch, size},
    test_pty::{WAIT, wait_until},
};
use crate::{
    adapter::{
        holder::{
            ClientMessage, ClientRole, HolderConfig, HolderControl, HolderError, HolderId,
            HolderMessage, HolderRecord, HolderServer, client::greet,
        },
        signal_target::SignalTarget,
    },
    domain::{
        color_scheme::ColorScheme,
        process::{StopPolicy, StopSignal},
        pty::SpawnRequest,
        value::{ByteSize, CommandLine, StopGrace},
    },
};

/// A holder serving a real child on a thread, with a supervisor connected
/// so it collects the exit and finishes cleanly.
pub(super) struct Holder {
    /// The holder's config.
    config: HolderConfig,
    /// The holder's thread, until it is joined.
    server: Option<JoinHandle<Result<(), HolderError>>>,
    /// The test's supervisor connection.
    supervisor: UnixStream,
    /// The scratch directory, removed on drop.
    _scratch: TempDir,
}

impl Holder {
    /// Starts a holder running `command` at the test terminal's size.
    pub(super) fn start(command: &str) -> Self {
        let scratch = scratch();
        let dir = scratch.path();
        let request = SpawnRequest::builder()
            .process(name())
            .command(CommandLine::try_new(command).unwrap())
            .project(Some(dir.to_path_buf()))
            .size(size(ROWS, COLS))
            .scrollback(SCROLLBACK)
            .log_max_size(ByteSize::try_new(RawByteSize::mib(1)).unwrap())
            .color_scheme(ColorScheme::Dark)
            .stop(
                StopPolicy::builder()
                    .signal(StopSignal::Terminate)
                    .grace_period(StopGrace::try_new(Duration::from_secs(5)).unwrap())
                    .build(),
            )
            .build();
        let config = HolderConfig::builder()
            .id(HolderId::generate().unwrap())
            .request(request)
            .socket(dir.join("sock").join("h.sock"))
            .record_dir(dir.join("records"))
            .log(dir.join("logs").join("api.log"))
            .supervisor_deadline(SUPERVISOR_DEADLINE)
            .build();
        let server_config = config.clone();
        let server = thread::spawn(move || HolderServer::run(server_config));
        let mut supervisor = None;
        wait_until("the holder socket", || {
            supervisor = UnixStream::connect(config.socket()).ok();
            supervisor.is_some()
        });
        let mut supervisor = supervisor.unwrap();
        greet(
            &mut supervisor,
            config.socket(),
            ClientRole::Supervisor,
            WAIT,
        )
        .unwrap();
        let mut drain = supervisor.try_clone().unwrap();
        drain.set_read_timeout(None).unwrap();
        thread::spawn(move || while HolderMessage::read_from(&mut drain).is_ok() {});
        Self {
            config,
            server: Some(server),
            supervisor,
            _scratch: scratch,
        }
    }

    /// The holder's socket.
    pub(super) fn socket(&self) -> PathBuf {
        self.config.socket().clone()
    }

    /// Writes `bytes` to the child as another, writable client would.
    pub(super) fn type_as_another_client(&self, bytes: &[u8]) {
        let mut client = UnixStream::connect(self.config.socket()).unwrap();
        greet(
            &mut client,
            self.config.socket(),
            ClientRole::Attach { read_only: false },
            WAIT,
        )
        .unwrap();
        ClientMessage::Input(bytes.to_vec())
            .write_to(&mut client)
            .unwrap();
    }

    /// Kills the child when it still runs, collects its exit and joins
    /// the holder.
    pub(super) fn finish(mut self) {
        ClientMessage::Control(HolderControl::Kill)
            .write_to(&mut self.supervisor)
            .unwrap();
        let server = self.server.take().unwrap();
        wait_until("the holder to finish", || {
            let _ = ClientMessage::CollectExit.write_to(&mut self.supervisor);
            thread::sleep(Duration::from_millis(20));
            server.is_finished()
        });
        server.join().unwrap().unwrap();
    }
}

impl Drop for Holder {
    /// Kills the child of a holder a failed test left running.
    fn drop(&mut self) {
        if self.server.is_some()
            && let Some(record) = HolderRecord::load_all(self.config.record_dir()).pop()
        {
            let _ = SignalTarget::Group(record.child_pid()).send(Signal::KILL);
        }
    }
}
