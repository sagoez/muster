use std::{
    fs,
    os::unix::net::UnixListener,
    path::{Path, PathBuf},
    process,
    time::{Duration, SystemTime},
};

use getset::Getters;
use muster_domain::{
    process::{StopPolicy, StopSignal},
    value::{CommandLine, Pid, ProcessName, ProjectRoot, StopGrace},
};
use muster_os::{state_location::StateLocation, test_scratch::short_scratch};
use muster_protocol::holder::{HolderDirs, HolderId, HolderRecord};
use muster_store::DATABASE_FILE;
use tempfile::TempDir;

use super::legacy_state_move::{LegacyStateMove, Rename};

/// Prefix of each scratch home, kept short so a holder socket under it fits
/// in a Unix socket address.
const SCRATCH_PREFIX: &str = "msm-";
/// The earlier muster's state directory inside the scratch home.
const LEGACY_DIR: &str = "old/muster";
/// The state directory it moves to, in folders not made yet.
pub(crate) const STATE_DIR: &str = "new/state/muster";
/// What the test database holds.
pub(crate) const DATABASE_BYTES: &[u8] = b"the user's stack";
/// The process the holder runs.
const PROCESS: &str = "web";
/// The command it runs.
const COMMAND: &str = "npm run dev";
/// What its log already holds.
const LOGGED: &[u8] = b"output\n";
/// Grace of its stop policy.
const STOP_GRACE: Duration = Duration::from_secs(1);

/// A scratch home holding an earlier muster's state directory, with a
/// database and one running holder: its socket listening, its record and its
/// log. Dropping it removes the home.
#[derive(Getters)]
pub(crate) struct LegacyHome {
    /// The scratch home.
    home: TempDir,
    /// The holder's socket, listening at the path it was launched with.
    listener: UnixListener,
    /// The holder's record as it saved it.
    #[getset(get = "pub(crate)")]
    record: HolderRecord,
}

impl LegacyHome {
    /// An earlier muster's state directory under a fresh scratch home.
    pub(crate) fn new() -> Self {
        let home = short_scratch(SCRATCH_PREFIX);
        let from = home.path().join(LEGACY_DIR);
        fs::create_dir_all(&from).unwrap();
        fs::write(from.join(DATABASE_FILE), DATABASE_BYTES).unwrap();
        let dirs = HolderDirs::under(&from);
        dirs.prepare_sockets().unwrap();
        let id = HolderId::generate().unwrap();
        let socket = dirs.socket_path(&id);
        let listener = UnixListener::bind(&socket).unwrap();
        listener.set_nonblocking(true).unwrap();
        let project = ProjectRoot::try_new(home.path().to_path_buf()).unwrap();
        let name = ProcessName::try_new(PROCESS).unwrap();
        let log = dirs.process_log(&project, &name);
        fs::create_dir_all(log.parent().unwrap()).unwrap();
        fs::write(&log, LOGGED).unwrap();
        let pid = Pid::try_new(process::id()).unwrap();
        let record = HolderRecord::builder()
            .id(id)
            .project(project)
            .process(name)
            .command(CommandLine::try_new(COMMAND).unwrap())
            .stop(
                StopPolicy::builder()
                    .signal(StopSignal::Terminate)
                    .grace_period(StopGrace::try_new(STOP_GRACE).unwrap())
                    .build(),
            )
            .socket(socket)
            .holder_pid(pid)
            .child_pid(pid)
            .child_start_token(None)
            .log(log)
            .child_started_at(SystemTime::UNIX_EPOCH)
            .build();
        record.save(dirs.records()).unwrap();
        Self {
            home,
            listener,
            record,
        }
    }

    /// The earlier muster's state directory.
    pub(crate) fn from(&self) -> PathBuf {
        self.home.path().join(LEGACY_DIR)
    }

    /// The state directory it moves to.
    pub(crate) fn to(&self) -> PathBuf {
        self.home.path().join(STATE_DIR)
    }

    /// The location whose state directory is the new one and whose legacy
    /// directory is the old one.
    pub(crate) fn location(&self) -> StateLocation {
        StateLocation::builder()
            .dir(self.to())
            .legacy(Some(self.from()))
            .build()
    }

    /// The move the location waits for.
    pub(crate) fn state_move(&self) -> LegacyStateMove {
        LegacyStateMove::pending(&self.location()).unwrap()
    }

    /// The move from the old directory to the new one, renaming through
    /// `rename`, which a test makes fail where it says.
    pub(crate) fn state_move_renaming(&self, rename: Rename) -> LegacyStateMove {
        LegacyStateMove::builder()
            .from(self.from())
            .to(self.to())
            .rename(rename)
            .build()
    }

    /// The move from the old directory to `to` instead.
    pub(crate) fn state_move_to(&self, to: &Path) -> LegacyStateMove {
        LegacyStateMove::builder()
            .from(self.from())
            .to(to.to_path_buf())
            .build()
    }

    /// Where the move stages the link to the new place.
    pub(crate) fn staged_link(&self) -> PathBuf {
        LegacyStateMove::builder()
            .from(self.from())
            .to(self.to())
            .build()
            .staged_link()
    }

    /// The records saved under the state directory `state_dir`.
    pub(crate) fn records_in(state_dir: &Path) -> Vec<HolderRecord> {
        HolderRecord::load_all(HolderDirs::under(state_dir).records())
    }

    /// Whether a connection reached the holder's socket since the last call.
    pub(crate) fn reached(&self) -> bool {
        self.listener.accept().is_ok()
    }
}
