use std::{
    path::{Path, PathBuf},
    process::{self, Command, Output},
    thread,
    time::Duration,
};

use muster::{
    adapter::{
        holder::{HOLDER_SUBCOMMAND, HolderDirs, HolderRecord},
        private_dir::{prepare_socket_dir, prepare_state_dir},
        service::{
            ServiceRequest, ServiceResponse,
            protocol::{self, SERVICE_SUBCOMMAND},
        },
        signal_target::SignalTarget,
        store::SqliteStore,
    },
    domain::{config::SettingChange, coordination::Author, port::SettingsStore},
};
use rustix::process::{Pid, Signal, kill_process, kill_process_group};
use tempfile::{Builder, TempDir};

use super::{herdr_env::without_herdr, processes::processes_under};

/// The muster binary cargo built for these tests.
pub const BINARY: &str = env!("CARGO_BIN_EXE_muster");
/// Short root for test sockets, inside `sun_path`.
const SHORT_TEMP_ROOT: &str = "/tmp";
/// Prefix of every scratch directory.
const SCRATCH_PREFIX: &str = "me-";
/// Directory muster keeps its state under, inside each XDG root.
const APP_DIR: &str = "muster";
/// The database file inside muster's state directory.
const DATABASE_FILE: &str = "muster.db";
/// Longest a service may take to answer one request a test sends.
const REPLY_TIMEOUT: Duration = Duration::from_secs(60);
/// Most rounds a scratch's teardown kills what is left under it.
const TEARDOWN_SWEEPS: usize = 50;
/// Pause between those rounds, for killed processes to go.
const TEARDOWN_PAUSE: Duration = Duration::from_millis(20);

/// Isolated XDG roots, a database with desktop notifications off, and holder
/// directories; dropping it kills every service, holder and child the test
/// left there, even on panic.
pub struct Scratch {
    /// The scratch directory, removed on drop.
    root: TempDir,
    /// Pids a test recorded to be killed with the scratch.
    children: Vec<u32>,
}

impl Scratch {
    /// A fresh scratch directory short enough to bind sockets in, holding a
    /// database with desktop notifications off.
    pub fn new() -> Self {
        let scratch = Self::empty();
        scratch
            .store()
            .apply(
                &SettingChange::DesktopNotifications(false),
                &Author::human(),
            )
            .unwrap();
        scratch
    }

    /// A fresh scratch directory short enough to bind sockets in, holding
    /// nothing yet.
    pub fn empty() -> Self {
        Self {
            root: Builder::new()
                .prefix(SCRATCH_PREFIX)
                .tempdir_in(SHORT_TEMP_ROOT)
                .unwrap(),
            children: Vec::new(),
        }
    }

    /// The scratch directory as created.
    pub fn root(&self) -> &Path {
        self.root.path()
    }

    /// The scratch directory with every link resolved.
    pub fn path(&self) -> PathBuf {
        self.root().canonicalize().unwrap()
    }

    /// The XDG state root muster runs under.
    pub fn state_root(&self) -> PathBuf {
        self.root().join("state")
    }

    /// muster's state directory.
    pub fn state_dir(&self) -> PathBuf {
        self.state_root().join(APP_DIR)
    }

    /// The XDG runtime directory muster runs under.
    pub fn runtime_dir(&self) -> PathBuf {
        self.root().join("run")
    }

    /// The socket a service over this scratch listens on.
    pub fn socket(&self) -> PathBuf {
        protocol::socket_path(&self.socket_dir(), &self.state_dir())
    }

    /// The directory that socket goes in.
    pub fn socket_dir(&self) -> PathBuf {
        self.runtime_dir().join(APP_DIR)
    }

    /// Creates the state directory and the socket directory private, as a
    /// service would, for a test that listens there itself.
    pub fn prepare_socket_dirs(&self) {
        prepare_state_dir(&self.state_dir()).unwrap();
        prepare_socket_dir(&self.socket_dir()).unwrap();
    }

    /// The database muster keeps here.
    pub fn database(&self) -> PathBuf {
        self.state_dir().join(DATABASE_FILE)
    }

    /// The database, opened.
    pub fn store(&self) -> SqliteStore {
        SqliteStore::open(&self.database()).unwrap()
    }

    /// Holder sockets, records and logs inside the scratch.
    pub fn holder_dirs(&self) -> HolderDirs {
        HolderDirs::builder()
            .sockets(self.root().join("sock"))
            .records(self.root().join("records"))
            .logs(self.root().join("logs"))
            .build()
    }

    /// `muster <args>` over this scratch's XDG roots, run from its root.
    pub fn command(&self, args: &[&str]) -> Command {
        let mut command = self.isolated(Path::new(BINARY));
        command.args(args);
        command
    }

    /// `program` over this scratch's XDG roots, run from its root, outside
    /// any herdr pane the suite runs in.
    pub fn isolated(&self, program: &Path) -> Command {
        let mut command = Command::new(program);
        without_herdr(&mut command)
            .current_dir(self.root())
            .env("XDG_STATE_HOME", self.state_root())
            .env("XDG_CONFIG_HOME", self.root().join("config"))
            .env("XDG_DATA_HOME", self.root().join("data"))
            .env("XDG_RUNTIME_DIR", self.runtime_dir())
            .env_remove("MUSTER_PROJECT");
        command
    }

    /// Runs `muster <args>` and returns its output.
    pub fn muster(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    /// Records `pid` to be killed with the scratch.
    pub fn keep_child(&mut self, pid: u32) {
        self.children.push(pid);
    }

    /// Sends `request` to the running service and returns the answer.
    pub fn send(&self, request: &ServiceRequest) -> ServiceResponse {
        protocol::send_within(&self.socket(), request, REPLY_TIMEOUT).unwrap()
    }
}

impl Drop for Scratch {
    /// Kills every service, holder, holder child and recorded child left
    /// under this scratch, even when the test failed midway. Services go
    /// first and the sweep repeats until nothing is left, since a service
    /// still alive would restart a killed child under a fresh holder.
    fn drop(&mut self) {
        for _ in 0..TEARDOWN_SWEEPS {
            let services = processes_under(SERVICE_SUBCOMMAND, self.root());
            for pid in self.children.iter().copied().chain(services) {
                kill_hard(pid);
            }
            let records = [self.holder_dirs(), HolderDirs::under(&self.state_dir())]
                .iter()
                .flat_map(|dirs| HolderRecord::load_all(dirs.records()))
                .collect::<Vec<_>>();
            for record in &records {
                if record.child_pid().into_inner() != process::id() {
                    let _ = SignalTarget::Group(record.child_pid()).send(Signal::KILL);
                }
            }
            let holders = processes_under(HOLDER_SUBCOMMAND, self.root());
            for pid in &holders {
                kill_hard(*pid);
            }
            let left = processes_under(SERVICE_SUBCOMMAND, self.root()).len()
                + processes_under(HOLDER_SUBCOMMAND, self.root()).len();
            if left == 0 {
                return;
            }
            thread::sleep(TEARDOWN_PAUSE);
        }
    }
}

/// Sends SIGKILL to the process `pid` and its group.
fn kill_hard(pid: u32) {
    if let Some(pid) = Pid::from_raw(pid as i32) {
        let _ = kill_process_group(pid, Signal::KILL);
        let _ = kill_process(pid, Signal::KILL);
    }
}
