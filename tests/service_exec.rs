//! The real service round trip: the built `muster` binary run as `__service`
//! over an isolated state directory, bringing a project up, exiting with its
//! holder still running, and a second service adopting that holder.

#![cfg(target_os = "linux")]

use std::{
    fs,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use muster::{
    adapter::{
        holder::HOLDER_SUBCOMMAND,
        service::{
            ProcessAction, ProcessStatus, ProjectStatus, ServiceRequest, ServiceResponse,
            protocol::{self, SERVICE_SUBCOMMAND},
        },
    },
    domain::{process::ProcessState, value::ProcessName},
};
use tempfile::TempDir;

/// The muster binary cargo built for this test.
const BINARY: &str = env!("CARGO_BIN_EXE_muster");
/// Bound on every wait; reaching it fails the test.
const WAIT: Duration = Duration::from_secs(10);
/// Interval between checks of a polled condition.
const POLL: Duration = Duration::from_millis(10);
/// Short root for test sockets, inside `sun_path`.
const SHORT_TEMP_ROOT: &str = "/tmp";
/// Prefix of the scratch directory.
const SCRATCH_PREFIX: &str = "mv-";
/// Directory muster keeps its state and config under, inside the XDG roots.
const APP_DIR: &str = "muster";
/// The settings file name inside muster's config directory.
const SETTINGS_FILE: &str = "settings.yml";
/// The settings muster ships, which every scratch starts from.
const SHIPPED_SETTINGS: &str = include_str!("../src/adapter/config/default_settings.yml");
/// The shipped desktop notification setting, and what scratches use instead.
const DESKTOP_ON: &str = "desktop_notifications: true";
const DESKTOP_OFF: &str = "desktop_notifications: false";
/// The setting the incomplete settings file leaves out.
const MISSING_SETTING: &str = "scrollback";
/// A settings file with every setting but [`MISSING_SETTING`].
const INCOMPLETE_SETTINGS: &str = "desktop_notifications: false\neditor:\n  command: \"\"\n  \
                                   terminal: \"\"\ndefault_size:\n  rows: 24\n  cols: 80\n\
                                   detach_key: \"ctrl-\\\\\"\nlog_max_size: 10MiB\n";
/// The project's one process, which runs until it is killed.
const PROJECT: &str = "agents: []\nterminals: []\ncommands:\n  - name: api\n    command: sleep 600\n    autostart: true\n";
/// A project whose one process ignores the hangup its holder's death sends.
const HUP_IGNORING_PROJECT: &str = "agents: []\nterminals: []\ncommands:\n  - name: api\n    command: trap '' HUP; sleep 600\n    autostart: true\n";
/// A project whose one process fails at once and is always restarted.
const FLAPPING_PROJECT: &str = "agents: []\nterminals: []\ncommands:\n  - name: api\n    command: exit 1\n    autostart: true\n    restart: always\n";
/// Restarts the flapper makes before its service is stopped.
const FLAPS_BEFORE_STOP: u32 = 2;
/// How long a test lets a refused start settle before checking it did nothing.
const SETTLE: Duration = Duration::from_millis(500);
/// How a crash notice starts.
const CRASHED: &str = "crashed";
/// How long a process runs before its service is replaced.
const UPTIME_BEFORE_STOP: Duration = Duration::from_millis(1500);

/// Polls `condition` until it holds, failing at [`WAIT`].
fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let started = Instant::now();
    while !condition() {
        assert!(started.elapsed() < WAIT, "timed out waiting for {what}");
        thread::sleep(POLL);
    }
}

/// Pids of processes whose command line names `subcommand` and mentions
/// `root`.
fn processes_under(subcommand: &str, root: &Path) -> Vec<u32> {
    let root = root.to_string_lossy().into_owned();
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .filter(|pid| {
            let cmdline = fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
            let environ = fs::read(format!("/proc/{pid}/environ")).unwrap_or_default();
            String::from_utf8_lossy(&cmdline).contains(subcommand)
                && String::from_utf8_lossy(&environ).contains(&root)
        })
        .collect()
}

/// Whether the process `pid` exists.
fn alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

/// Isolated XDG roots and a project; dropping it kills every service, holder
/// and child the test left there, even on panic.
struct Scratch {
    root: TempDir,
    children: Vec<u32>,
}

impl Scratch {
    /// A fresh scratch directory short enough to bind sockets in, with the
    /// shipped settings but desktop notifications off.
    fn new() -> Self {
        let root = tempfile::Builder::new()
            .prefix(SCRATCH_PREFIX)
            .tempdir_in(SHORT_TEMP_ROOT)
            .unwrap();
        let settings = root.path().join("config").join(APP_DIR).join(SETTINGS_FILE);
        fs::create_dir_all(settings.parent().unwrap()).unwrap();
        fs::write(&settings, SHIPPED_SETTINGS.replace(DESKTOP_ON, DESKTOP_OFF)).unwrap();
        Self {
            root,
            children: Vec::new(),
        }
    }

    /// The XDG state root the services run under.
    fn state_root(&self) -> PathBuf {
        self.root.path().join("state")
    }

    /// The socket a service over this scratch listens on.
    fn socket(&self) -> PathBuf {
        protocol::socket_path(&self.state_root().join(APP_DIR))
    }

    /// Writes the project's `muster.yml` and returns its path.
    fn project(&self) -> PathBuf {
        self.project_with(PROJECT)
    }

    /// Writes `config` as the project's `muster.yml` and returns its path.
    fn project_with(&self, config: &str) -> PathBuf {
        let dir = self.root.path().join("project");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("muster.yml");
        fs::write(&path, config).unwrap();
        path
    }

    /// The pids of the holders running under this scratch.
    fn holders(&self) -> Vec<u32> {
        processes_under(HOLDER_SUBCOMMAND, self.root.path())
    }

    /// The one process `ps` reports for `project`.
    fn api(&self, project: &Path) -> ProcessStatus {
        self.status(project).processes()[0].clone()
    }

    /// Brings `project` up and waits until its one process runs, returning
    /// its status.
    fn up_running(&mut self, project: &Path) -> ProcessStatus {
        self.send(ServiceRequest::Up {
            project: project.to_path_buf(),
        });
        wait_until("the process to run", || {
            *self.api(project).state() == ProcessState::Running
        });
        let api = self.api(project);
        self.children.push(api.pid().unwrap());
        api
    }

    /// Sends `action` for `api` in `project`.
    fn control(&self, project: &Path, action: ProcessAction) -> ServiceResponse {
        self.send(ServiceRequest::Control {
            project: project.to_path_buf(),
            name: ProcessName::try_new("api").unwrap(),
            action,
        })
    }

    /// The recent notices of `project`'s processes, as text.
    fn notices(&self, project: &Path) -> Vec<String> {
        let ServiceResponse::Notices(notices) = self.send(ServiceRequest::Notices {
            project: project.to_path_buf(),
        }) else {
            panic!("notices answer notices");
        };
        notices
            .iter()
            .map(|notice| notice.message().clone())
            .collect()
    }

    /// The XDG config root the services read their settings from.
    fn config_root(&self) -> PathBuf {
        self.root.path().join("config")
    }

    /// Spawns `muster __service` over this scratch's XDG roots with no
    /// terminal, as a client starts it.
    fn spawn_service(&self) -> Child {
        Command::new(BINARY)
            .arg(SERVICE_SUBCOMMAND)
            .env("XDG_STATE_HOME", self.state_root())
            .env("XDG_CONFIG_HOME", self.config_root())
            .env("XDG_DATA_HOME", self.root.path().join("data"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    }

    /// Starts `muster __service` over this scratch's XDG roots and waits until
    /// it accepts.
    fn start_service(&self) -> Child {
        let child = self.spawn_service();
        let socket = self.socket();
        wait_until("the service to accept", || {
            UnixStream::connect(&socket).is_ok()
        });
        child
    }

    /// Sends `request` to the running service and returns the answer.
    fn send(&self, request: ServiceRequest) -> ServiceResponse {
        protocol::send(&self.socket(), &request).unwrap()
    }

    /// The one project status a `ps` of `project` reports.
    fn status(&self, project: &Path) -> ProjectStatus {
        let ServiceResponse::Status(mut statuses) = self.send(ServiceRequest::Ps {
            project: Some(project.to_path_buf()),
        }) else {
            panic!("ps answers a status");
        };
        statuses.remove(0)
    }

    /// Stops the service `child` and waits until it has exited cleanly.
    fn stop_service(&self, mut child: Child) {
        assert_eq!(self.send(ServiceRequest::Stop), ServiceResponse::Ok);
        let mut status = None;
        wait_until("the service to exit", || {
            status = child.try_wait().unwrap();
            status.is_some()
        });
        assert!(status.unwrap().success());
    }
}

impl Drop for Scratch {
    /// Kills every service, holder and recorded child left under this scratch.
    fn drop(&mut self) {
        for pid in self
            .children
            .iter()
            .copied()
            .chain(processes_under(SERVICE_SUBCOMMAND, self.root.path()))
            .chain(processes_under(HOLDER_SUBCOMMAND, self.root.path()))
        {
            // SAFETY: `kill` takes plain integers and touches no memory.
            unsafe {
                libc::kill(-(pid as libc::pid_t), libc::SIGKILL);
                libc::kill(pid as libc::pid_t, libc::SIGKILL);
            }
        }
    }
}

/// Sends SIGKILL to the process `pid`, as a crash or the OOM killer would.
fn kill_hard(pid: u32) {
    // SAFETY: `kill` takes plain integers and touches no memory.
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGKILL);
    }
}

/// A child that outlives its SIGKILLed holder, as one that ignores the hangup
/// does, is left unsupervised: `start` never runs a second copy beside it,
/// neither under the service that saw the holder die nor under a successor.
#[test]
fn a_child_outliving_its_killed_holder_is_unsupervised_and_never_started_twice() {
    let mut scratch = Scratch::new();
    let project = scratch.project_with(HUP_IGNORING_PROJECT);
    let service = scratch.start_service();
    let pid = *scratch.up_running(&project).pid();
    let holders = scratch.holders();
    assert_eq!(holders.len(), 1);

    kill_hard(holders[0]);
    wait_until("the process to be unsupervised", || {
        *scratch.api(&project).state() == ProcessState::Unsupervised
    });
    assert!(alive(pid.unwrap()), "the child outlived its holder");
    scratch.control(&project, ProcessAction::Start);
    thread::sleep(SETTLE);
    assert_eq!(*scratch.api(&project).state(), ProcessState::Unsupervised);
    assert!(scratch.holders().is_empty(), "no second copy was started");

    scratch.stop_service(service);
    let successor = scratch.start_service();
    wait_until("the successor to find the child unsupervised", || {
        *scratch.api(&project).state() == ProcessState::Unsupervised
    });
    scratch.control(&project, ProcessAction::Start);
    thread::sleep(SETTLE);
    assert_eq!(*scratch.api(&project).state(), ProcessState::Unsupervised);
    assert!(scratch.holders().is_empty(), "no second copy was started");
    assert_eq!(*scratch.api(&project).pid(), pid);

    kill_hard(pid.unwrap());
    scratch.stop_service(successor);
}

/// A holder SIGKILLed with its child, whose exit status nobody saw, is still
/// told as a crash.
#[test]
fn a_holder_killed_with_its_child_is_told_as_a_crash() {
    let mut scratch = Scratch::new();
    let project = scratch.project();
    let service = scratch.start_service();
    scratch.up_running(&project);

    kill_hard(scratch.holders()[0]);
    wait_until("the crash to be told", || {
        scratch
            .notices(&project)
            .iter()
            .any(|notice| notice.starts_with(CRASHED))
    });
    assert!(!scratch.api(&project).state().may_be_running());
    scratch.stop_service(service);
}

/// A process restarting under `restart: always` when the service stops keeps
/// restarting under the successor, which also keeps its restart count.
#[test]
fn a_restart_pending_at_service_stop_is_carried_out_by_the_successor() {
    let scratch = Scratch::new();
    let project = scratch.project_with(FLAPPING_PROJECT);
    let service = scratch.start_service();
    scratch.send(ServiceRequest::Up {
        project: project.clone(),
    });
    wait_until("the flapper to restart", || {
        *scratch.api(&project).restarts() >= FLAPS_BEFORE_STOP
    });

    let before = *scratch.api(&project).restarts();
    scratch.stop_service(service);
    assert!(
        !scratch.holders().is_empty(),
        "the exited child's holder keeps its exit for the successor"
    );

    let successor = scratch.start_service();
    wait_until("the successor to adopt the flapper", || {
        matches!(
            scratch.api(&project).state(),
            ProcessState::Running | ProcessState::Restarting
        )
    });
    let carried = *scratch.api(&project).restarts();
    assert!(
        carried >= before,
        "the successor kept the count: {carried} after {before}"
    );
    wait_until("the successor to restart it again", || {
        *scratch.api(&project).restarts() > carried
    });
    scratch.control(&project, ProcessAction::Stop);
    wait_until("the stop to settle", || {
        !scratch.api(&project).state().may_be_running()
    });
    scratch.stop_service(successor);
}

/// An adopted process keeps the uptime it had, rather than starting over.
#[test]
fn an_adopted_process_keeps_its_uptime() {
    let mut scratch = Scratch::new();
    let project = scratch.project();
    let service = scratch.start_service();
    scratch.up_running(&project);
    thread::sleep(UPTIME_BEFORE_STOP);

    scratch.stop_service(service);
    let successor = scratch.start_service();
    wait_until("the adoption to settle", || {
        *scratch.api(&project).state() == ProcessState::Running
    });
    let uptime = scratch.api(&project).uptime().unwrap();
    assert!(uptime >= UPTIME_BEFORE_STOP, "uptime {uptime:?}");
    scratch.control(&project, ProcessAction::Kill);
    wait_until("the kill to settle", || {
        !scratch.api(&project).state().may_be_running()
    });
    scratch.stop_service(successor);
}

/// A service that cannot start, here for a settings file missing a key,
/// exits with a failure that its log records, naming the file and the key,
/// although it was started with no terminal.
#[test]
fn a_service_that_cannot_start_says_why_in_its_log() {
    let scratch = Scratch::new();
    let settings = scratch.config_root().join(APP_DIR).join(SETTINGS_FILE);
    fs::create_dir_all(settings.parent().unwrap()).unwrap();
    fs::write(&settings, INCOMPLETE_SETTINGS).unwrap();

    let mut service = scratch.spawn_service();
    let mut status = None;
    wait_until("the service to give up", || {
        status = service.try_wait().unwrap();
        status.is_some()
    });

    assert!(!status.unwrap().success());
    let log = fs::read_to_string(protocol::log_path(&scratch.state_root().join(APP_DIR))).unwrap();
    assert!(log.contains(&settings.display().to_string()), "{log}");
    assert!(log.contains(MISSING_SETTING), "{log}");
}

#[test]
fn a_service_exits_leaving_its_holder_running_and_a_successor_adopts_it() {
    let mut scratch = Scratch::new();
    let project = scratch.project();
    let service = scratch.start_service();

    let ServiceResponse::Batch(up) = scratch.send(ServiceRequest::Up {
        project: project.clone(),
    }) else {
        panic!("up answers what it started");
    };
    assert_eq!(up.acted().len(), 1, "{up:?}");
    let mut api = scratch.status(&project).processes()[0].clone();
    wait_until("the process to run", || {
        api = scratch.status(&project).processes()[0].clone();
        *api.state() == ProcessState::Running
    });
    let pid = api.pid().unwrap();
    scratch.children.push(pid);

    scratch.stop_service(service);

    assert!(!scratch.socket().exists(), "the socket is removed");
    assert!(alive(pid), "the process outlives the service");
    assert_eq!(
        processes_under(HOLDER_SUBCOMMAND, scratch.root.path()).len(),
        1
    );

    let successor = scratch.start_service();
    let ServiceResponse::Status(loaded) = scratch.send(ServiceRequest::Ps { project: None }) else {
        panic!("ps answers a status");
    };
    assert_eq!(
        loaded.len(),
        1,
        "the successor loaded the project to adopt it"
    );
    wait_until("the adoption to settle", || {
        *scratch.status(&project).processes()[0].state() == ProcessState::Running
    });
    assert_eq!(*scratch.status(&project).processes()[0].pid(), Some(pid));

    scratch.send(ServiceRequest::Control {
        project: project.clone(),
        name: ProcessName::try_new("api").unwrap(),
        action: ProcessAction::Kill,
    });
    wait_until("the kill to settle", || {
        !scratch.status(&project).processes()[0]
            .state()
            .may_be_running()
    });
    scratch.stop_service(successor);
    wait_until("the holder to finish", || {
        processes_under(HOLDER_SUBCOMMAND, scratch.root.path()).is_empty()
    });
    assert!(!alive(pid));
}
