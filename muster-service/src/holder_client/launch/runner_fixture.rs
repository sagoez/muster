use std::{
    fs, mem,
    path::Path,
    process,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use muster_domain::{pty::SpawnRequest, value::ProjectRoot};
use muster_holder::{holders::Holders, testing::test_request, thread_launcher::ThreadLauncher};
use muster_os::{
    signal_target::SignalTarget,
    test_scratch::short_scratch,
    test_wait::{POLL, WAIT},
};
use muster_protocol::holder::{HolderConfig, HolderDirs, HolderId, HolderLauncher, HolderRecord};
use rustix::process::Signal;
use tempfile::TempDir;

use super::{
    runner::{HolderRunner, abandon},
    sabotage::Sabotage,
    spec_launcher::SpecLauncher,
};

/// Prefix of every runner test's scratch directory.
const SCRATCH_PREFIX: &str = "mr-";
/// The scratch's stand-in for the service's socket directory.
const RUNTIME_DIR: &str = "run";
/// The holder sockets' folder in that directory.
const RUNTIME_SOCKETS: &str = "holders";
/// The character a too-deep state directory's name repeats.
const DEEP_STATE_CHAR: &str = "d";
/// Characters in a state folder name that leave no room for a holder socket.
const DEEP_STATE_DIR_CHARS: usize = 120;

/// Holder dirs in a scratch directory, plus every holder thread started
/// there; dropping it kills and collects every child left running.
pub(super) struct Fixture {
    dirs: HolderDirs,
    holders: Holders,
    scratch: TempDir,
}

impl Fixture {
    /// Fresh holder dirs in a new scratch directory, binding sockets in its
    /// `sock` folder, with a `run` folder as the runtime one.
    pub(super) fn new() -> Self {
        Self::over(|root| {
            let sockets = root.join("sock");
            HolderDirs::builder()
                .sockets(sockets.clone())
                .state_sockets(sockets)
                .runtime_sockets(root.join(RUNTIME_DIR).join(RUNTIME_SOCKETS))
                .records(root.join("records"))
                .logs(root.join("logs"))
                .build()
        })
    }

    /// Fresh holder dirs of a state directory in a new scratch directory too
    /// deep for a holder socket, so its holders bind in the runtime folder.
    pub(super) fn with_deep_state_dir() -> Self {
        Self::over(|root| {
            HolderDirs::placed(
                &root.join(DEEP_STATE_CHAR.repeat(DEEP_STATE_DIR_CHARS)),
                &root.join(RUNTIME_DIR),
            )
        })
    }

    /// Fresh holder dirs `dirs` places in a new scratch directory.
    fn over(dirs: impl FnOnce(&Path) -> HolderDirs) -> Self {
        let scratch = short_scratch(SCRATCH_PREFIX);
        let dirs = dirs(scratch.path());
        Self {
            dirs,
            holders: Arc::default(),
            scratch,
        }
    }

    /// The holder dirs every runner of this fixture uses.
    pub(super) fn dirs(&self) -> &HolderDirs {
        &self.dirs
    }

    /// Every holder thread this fixture's launchers started.
    pub(super) fn holders(&self) -> &Holders {
        &self.holders
    }

    /// The scratch directory, which is also the test project.
    pub(super) fn scratch(&self) -> &TempDir {
        &self.scratch
    }

    /// A fresh runner over the same dirs, as a restarted service has.
    pub(super) fn runner(&self) -> HolderRunner {
        self.runner_with(Box::new(ThreadLauncher::new(Arc::clone(&self.holders))))
    }

    /// A runner over this fixture's dirs launching through `launcher`.
    pub(super) fn runner_with(&self, launcher: Box<dyn HolderLauncher>) -> HolderRunner {
        self.runner_over(self.dirs.clone(), launcher)
    }

    /// A runner over `dirs` launching through `launcher`.
    pub(super) fn runner_over(
        &self,
        dirs: HolderDirs,
        launcher: Box<dyn HolderLauncher>,
    ) -> HolderRunner {
        HolderRunner::new(launcher, dirs)
    }

    /// The project every test child belongs to: the scratch directory.
    pub(super) fn root(&self) -> ProjectRoot {
        ProjectRoot::try_new(self.scratch.path().to_path_buf()).unwrap()
    }

    /// A launcher running holders from spec files, sabotaged as given.
    pub(super) fn spec_launcher(&self, sabotage: Option<Sabotage>) -> Box<dyn HolderLauncher> {
        Box::new(SpecLauncher::new(Arc::clone(&self.holders), sabotage))
    }

    /// A holder config for `command` as the runner would build it, with
    /// `supervisor_deadline`.
    pub(super) fn config(&self, command: &str, supervisor_deadline: Duration) -> HolderConfig {
        let id = HolderId::generate().unwrap();
        let request = self.request(command);
        HolderConfig::builder()
            .log(self.dirs.process_log(&self.root(), request.process()))
            .socket(self.dirs.socket_path(&id))
            .id(id)
            .request(request)
            .record_dir(self.dirs.records().clone())
            .supervisor_deadline(supervisor_deadline)
            .build()
    }

    /// A request to run `command` as `dev server` of this fixture's project.
    pub(super) fn request(&self, command: &str) -> SpawnRequest {
        test_request(
            "dev server",
            command,
            Some(self.scratch.path().to_path_buf()),
            None,
        )
    }

    /// Every holder record saved in this fixture's record directory.
    pub(super) fn record_files(&self) -> Vec<HolderRecord> {
        HolderRecord::load_all(self.dirs.records())
    }

    /// The text of the log `record` names, empty when it cannot be read.
    pub(super) fn log(&self, record: &HolderRecord) -> String {
        String::from_utf8_lossy(&fs::read(record.log()).unwrap_or_default()).into_owned()
    }
}

impl Drop for Fixture {
    /// Kills every child still recorded, collects its exit so its holder
    /// returns, and joins every holder thread, all bounded by [`WAIT`]. A
    /// record naming the test process itself (a stale-record test's stand-in)
    /// is never signalled.
    fn drop(&mut self) {
        for record in self.record_files() {
            if record.child_pid().into_inner() != process::id() {
                let _ = SignalTarget::Group(record.child_pid()).send(Signal::KILL);
            }
            abandon(record.socket());
        }
        let started = Instant::now();
        let holders = mem::take(&mut *self.holders.lock().unwrap());
        for holder in holders {
            while !holder.is_finished() && started.elapsed() < WAIT {
                thread::sleep(POLL);
            }
            if holder.is_finished() {
                let _ = holder.join();
            }
        }
    }
}
