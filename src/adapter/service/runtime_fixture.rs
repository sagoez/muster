use std::{
    fs, mem, os::unix::net::UnixStream, path::PathBuf, process, sync::Arc, thread, time::Instant,
};

use crossbeam_channel::Receiver;
use rustix::process::Signal;
use tempfile::TempDir;

use super::{
    holder_pool::HolderPool,
    memory_store::MemoryStore,
    protocol,
    running::Running,
    runtime::ServiceRuntime,
    runtime_testing::{POLL, SCRATCH_PREFIX, TEST_BINARY, VERSION, WAIT, wait_until},
    service_deps::ServiceDeps,
    singleton::Singleton,
    test_ports::TestPorts,
};
use crate::{
    adapter::{
        holder::{
            HolderDirs, HolderRecord, HolderRunner, abandon, holders::Holders,
            testing::test_request, thread_launcher::ThreadLauncher,
        },
        signal_target::SignalTarget,
        test_scratch::short_scratch,
    },
    domain::{config::ProcessSpec, pty::SpawnRequest, value::ProjectRoot},
};

/// A scratch state directory, holder dirs, projects and their store, plus
/// every holder thread started there; dropping it kills every child still
/// recorded and lets its holder finish.
pub(super) struct Fixture {
    pub(super) scratch: TempDir,
    pub(super) holders: Holders,
    pub(super) store: MemoryStore,
}

impl Fixture {
    /// A fresh scratch directory short enough to bind sockets in.
    pub(super) fn new() -> Self {
        Self {
            scratch: short_scratch(SCRATCH_PREFIX),
            holders: Arc::default(),
            store: MemoryStore::new(),
        }
    }

    /// The state directory the test services own.
    pub(super) fn state_dir(&self) -> PathBuf {
        self.scratch.path().join("state")
    }

    /// Where this fixture's holders keep their sockets, records and logs. The
    /// logs sit beside the holders' folder, as a service's do, since a
    /// service starting removes a `logs` folder inside it as legacy.
    pub(super) fn dirs(&self) -> HolderDirs {
        let root = self.scratch.path().join("h");
        HolderDirs::builder()
            .sockets(root.join("sock"))
            .records(root.join("records"))
            .logs(self.scratch.path().join("logs"))
            .build()
    }

    /// A runner launching in-thread holders into this fixture's dirs, as a
    /// service has.
    pub(super) fn runner(&self) -> HolderRunner {
        HolderRunner::new(
            Box::new(ThreadLauncher::new(Arc::clone(&self.holders))),
            self.dirs(),
        )
    }

    /// As [`Self::runner`], whose holders start only once `gate` opens.
    pub(super) fn gated_runner(&self, gate: Receiver<()>) -> HolderRunner {
        HolderRunner::new(
            Box::new(ThreadLauncher::gated(Arc::clone(&self.holders), gate, WAIT)),
            self.dirs(),
        )
    }

    /// The folder `folder` in the scratch directory, created.
    pub(super) fn folder(&self, folder: &str) -> ProjectRoot {
        let dir = self.scratch.path().join(folder);
        fs::create_dir_all(&dir).unwrap();
        ProjectRoot::try_new(fs::canonicalize(dir).unwrap()).unwrap()
    }

    /// Registers the project `project` with `processes` and returns its
    /// folder.
    pub(super) fn project(&self, processes: Vec<ProcessSpec>) -> ProjectRoot {
        self.project_in("project", processes)
    }

    /// Registers the project in `folder` with `processes` and returns it.
    pub(super) fn project_in(&self, folder: &str, processes: Vec<ProcessSpec>) -> ProjectRoot {
        let root = self.folder(folder);
        self.store.put(&root, processes);
        root
    }

    /// A request for `process` running `command` in `project`.
    pub(super) fn request(
        &self,
        project: &ProjectRoot,
        process: &str,
        command: &str,
    ) -> SpawnRequest {
        test_request(
            process,
            command,
            Some(project.as_ref().clone()),
            Some(self.scratch.path().to_path_buf()),
        )
    }

    /// Starts a service over `pool` and waits until it accepts clients.
    pub(super) fn start(&self, pool: Arc<dyn HolderPool>) -> Running {
        let state_dir = self.state_dir();
        let socket = protocol::socket_path(&state_dir);
        let store = self.store.clone();
        let thread = thread::spawn(move || {
            let Some(singleton) = Singleton::acquire(&state_dir)? else {
                return Ok(());
            };
            let deps = ServiceDeps::builder()
                .version(VERSION.parse().unwrap())
                .binary(PathBuf::from(TEST_BINARY))
                .holders(pool)
                .ports(Box::new(TestPorts))
                .metrics(None)
                .stacks(Box::new(store.clone()))
                .settings(Box::new(store.clone()))
                .runs(Arc::new(store.clone()))
                .raised(Arc::new(store.clone()))
                .edits(Box::new(store))
                .listening(None)
                .build();
            ServiceRuntime::run(singleton, deps)
        });
        wait_until("the service to accept", || {
            UnixStream::connect(&socket).is_ok()
        });
        Running {
            socket,
            thread: Some(thread),
        }
    }
}

impl Drop for Fixture {
    /// Kills every child still recorded and collects its exit so its
    /// holder finishes, then joins the holder threads, bounded by [`WAIT`].
    fn drop(&mut self) {
        let dirs = self.dirs();
        for record in HolderRecord::load_all(dirs.records()) {
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
