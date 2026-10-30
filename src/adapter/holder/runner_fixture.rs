use std::{
    fs, mem, process,
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

use rustix::process::Signal;
use tempfile::TempDir;

use super::{
    config::HolderConfig,
    dirs::HolderDirs,
    holder_launcher::HolderLauncher,
    holders::Holders,
    id::HolderId,
    record::HolderRecord,
    runner::{HolderRunner, abandon},
    sabotage::Sabotage,
    server_testing::{POLL, WAIT},
    spec_launcher::SpecLauncher,
    testing::test_request,
    thread_launcher::ThreadLauncher,
};
use crate::{
    adapter::{signal_target::SignalTarget, test_scratch::short_scratch},
    domain::{pty::SpawnRequest, value::ProjectRoot},
};

/// Prefix of every runner test's scratch directory.
const SCRATCH_PREFIX: &str = "mr-";

/// Holder dirs in a scratch directory, plus every holder thread started
/// there; dropping it kills and collects every child left running.
pub(super) struct Fixture {
    dirs: HolderDirs,
    holders: Holders,
    scratch: TempDir,
}

impl Fixture {
    /// Fresh holder dirs in a new scratch directory.
    pub(super) fn new() -> Self {
        let scratch = short_scratch(SCRATCH_PREFIX);
        let root = scratch.path();
        let dirs = HolderDirs::builder()
            .sockets(root.join("sock"))
            .records(root.join("records"))
            .logs(root.join("logs"))
            .build();
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
