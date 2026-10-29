//! The real service round trip: the built `muster` binary run as `__service`
//! over an isolated state directory whose database holds the project's stack,
//! bringing the project up, following stack and settings changes, exiting
//! with its holder still running, and a second service adopting that holder.

#![cfg(target_os = "linux")]

use std::{
    ffi::OsString,
    fs, io,
    os::unix::{net::UnixStream, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use muster::{
    adapter::{
        holder::HOLDER_SUBCOMMAND,
        service::{
            LaunchEnvironment, ProcessAction, ProcessStatus, ProjectStatus, ServiceRequest,
            ServiceResponse,
            protocol::{self, SERVICE_SUBCOMMAND},
        },
        store::SqliteStore,
    },
    domain::{
        config::{Environment, ProcessSpec, SettingChange},
        port::{SettingsStore, StackStore},
        process::{ProcessState, RestartPolicy, StopPolicy, StopSignal},
        value::{CommandLine, Pid, ProcessName, ProjectRoot, StopGrace},
    },
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
/// Directory muster keeps its state under, inside the XDG state root.
const APP_DIR: &str = "muster";
/// The database file inside muster's state directory.
const DATABASE_FILE: &str = "muster.db";
/// The project's one process, which runs until it is killed.
const SLEEPER: &str = "sleep 600";
/// A process that ignores the hangup its holder's death sends.
const HUP_IGNORING: &str = "trap '' HUP; sleep 600";
/// The state `/proc/<pid>/stat` gives a process that died unreaped.
const ZOMBIE_STATE: &str = "Z";
/// A process that fails at once.
const FLAPPER: &str = "exit 1";
/// Restarts the flapper makes before its service is stopped.
const FLAPS_BEFORE_STOP: u32 = 2;
/// How long a test lets a refused start settle before checking it did nothing.
const SETTLE: Duration = Duration::from_millis(500);
/// How a crash notice starts.
const CRASHED: &str = "crashed";
/// How long a process runs before its service is replaced.
const UPTIME_BEFORE_STOP: Duration = Duration::from_millis(1500);
/// Grace every test process's stop policy gives it.
const STOP_GRACE: Duration = Duration::from_secs(2);
/// A variable the service inherits that no child may see.
const LEAKED_VAR: &str = "MUSTER_TEST_SERVICE_SECRET";
/// A variable the service inherits outside the baseline, which no child may
/// see either.
const INHERITED_VAR: &str = "SERVICE_ONLY_TOKEN";
/// A variable of the client that asked for a launch.
const CLIENT_VAR: &str = "FROM_CLIENT";
/// Its value.
const CLIENT_VALUE: &str = "client-value";
/// The notice of a holder found running whose process left the project.
const REMOVED_NOTICE: &str = "stopped: removed from the project";
/// The row of `/proc/<pid>/limits` for open files.
const OPEN_FILES_ROW: &str = "Max open files";
/// The open-files soft limit a service is started with, below the hard limit
/// `sysinfo` would raise it to.
const SERVICE_OPEN_FILES: libc::rlim_t = 1000;

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

/// This process's soft and hard limits on open files.
fn open_files_limits() -> libc::rlimit {
    let mut limits = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `getrlimit` only writes the struct it is handed.
    unsafe {
        libc::getrlimit(libc::RLIMIT_NOFILE, &mut limits);
    }
    limits
}

/// The soft limit on open files of the process `pid`.
fn soft_open_files(pid: u32) -> String {
    fs::read_to_string(format!("/proc/{pid}/limits"))
        .unwrap()
        .lines()
        .find_map(|line| line.strip_prefix(OPEN_FILES_ROW))
        .and_then(|limits| limits.split_whitespace().next())
        .unwrap()
        .to_string()
}

/// Whether the process `pid` exists.
fn alive(pid: u32) -> bool {
    Path::new(&format!("/proc/{pid}")).exists()
}

/// The environment of the process `pid`, as `NAME=value` entries.
fn environ(pid: u32) -> Vec<String> {
    fs::read(format!("/proc/{pid}/environ"))
        .unwrap_or_default()
        .split(|byte| *byte == 0)
        .map(|entry| String::from_utf8_lossy(entry).into_owned())
        .collect()
}

/// A process `name` running `command` under `restart`, starting with the
/// project.
fn process(name: &str, command: &str, restart: RestartPolicy) -> ProcessSpec {
    ProcessSpec::builder()
        .name(ProcessName::try_new(name).unwrap())
        .command(CommandLine::try_new(command).unwrap())
        .environment(Environment::empty())
        .autostart(true)
        .restart(restart)
        .stop(
            StopPolicy::builder()
                .signal(StopSignal::Terminate)
                .grace_period(StopGrace::try_new(STOP_GRACE).unwrap())
                .build(),
        )
        .build()
}

/// The client environment carrying [`CLIENT_VAR`].
fn client_env() -> LaunchEnvironment {
    LaunchEnvironment::from_vars([(OsString::from(CLIENT_VAR), OsString::from(CLIENT_VALUE))])
}

/// Isolated XDG roots, a database and a project; dropping it kills every
/// service, holder and child the test left there, even on panic.
struct Scratch {
    root: TempDir,
    children: Vec<u32>,
}

impl Scratch {
    /// A fresh scratch directory short enough to bind sockets in, whose
    /// database has desktop notifications off.
    fn new() -> Self {
        let root = tempfile::Builder::new()
            .prefix(SCRATCH_PREFIX)
            .tempdir_in(SHORT_TEMP_ROOT)
            .unwrap();
        let scratch = Self {
            root,
            children: Vec::new(),
        };
        scratch
            .store()
            .apply(&SettingChange::DesktopNotifications(false))
            .unwrap();
        scratch
    }

    /// The XDG state root the services run under.
    fn state_root(&self) -> PathBuf {
        self.root.path().join("state")
    }

    /// The database the services read.
    fn store(&self) -> SqliteStore {
        SqliteStore::open(&self.state_root().join(APP_DIR).join(DATABASE_FILE)).unwrap()
    }

    /// The socket a service over this scratch listens on.
    fn socket(&self) -> PathBuf {
        protocol::socket_path(&self.state_root().join(APP_DIR))
    }

    /// Registers the project with one process `api` running [`SLEEPER`].
    fn project(&self) -> ProjectRoot {
        self.project_with(&[process("api", SLEEPER, RestartPolicy::Never)])
    }

    /// Registers the project with `processes` and returns its folder.
    fn project_with(&self, processes: &[ProcessSpec]) -> ProjectRoot {
        let dir = self.root.path().join("project");
        fs::create_dir_all(&dir).unwrap();
        let root = ProjectRoot::try_new(fs::canonicalize(dir).unwrap()).unwrap();
        let store = self.store();
        for spec in processes {
            store.add_process(&root, spec).unwrap();
        }
        root
    }

    /// The pids of the holders running under this scratch.
    fn holders(&self) -> Vec<u32> {
        processes_under(HOLDER_SUBCOMMAND, self.root.path())
    }

    /// The status `ps` reports for the process `name` of `project`.
    fn named(&self, project: &ProjectRoot, name: &str) -> Option<ProcessStatus> {
        self.status(project)
            .processes()
            .iter()
            .find(|process| process.name().as_ref() == name)
            .cloned()
    }

    /// The status `ps` reports for `api` of `project`.
    fn api(&self, project: &ProjectRoot) -> ProcessStatus {
        self.named(project, "api").unwrap()
    }

    /// Brings `project` up as a client with [`client_env`], and waits until
    /// `api` runs, returning its status.
    fn up_running(&mut self, project: &ProjectRoot) -> ProcessStatus {
        self.up(project);
        wait_until("the process to run", || {
            *self.api(project).state() == ProcessState::Running
        });
        let api = self.api(project);
        self.children.push(api.pid().unwrap().into_inner());
        api
    }

    /// Brings `project` up as a client with [`client_env`].
    fn up(&self, project: &ProjectRoot) -> ServiceResponse {
        self.send(ServiceRequest::Up {
            project: project.clone(),
            environment: client_env(),
            wait: None,
        })
    }

    /// Sends `action` for `api` in `project`.
    fn control(&self, project: &ProjectRoot, action: ProcessAction) -> ServiceResponse {
        self.send(ServiceRequest::Control {
            project: project.clone(),
            name: ProcessName::try_new("api").unwrap(),
            action,
            environment: client_env(),
            wait: None,
        })
    }

    /// The recent notices of `project`, as `(process, message)`.
    fn notices(&self, project: &ProjectRoot) -> Vec<(Option<String>, String)> {
        let ServiceResponse::Notices(notices) = self.send(ServiceRequest::Notices {
            project: project.clone(),
        }) else {
            panic!("notices answer notices");
        };
        notices
            .iter()
            .map(|notice| {
                (
                    notice.process().as_ref().map(ToString::to_string),
                    notice.message().clone(),
                )
            })
            .collect()
    }

    /// Spawns `muster __service` over this scratch's XDG roots with no
    /// terminal, as a client starts it, carrying variables no child may see.
    fn spawn_service(&self) -> Child {
        self.service_command().spawn().unwrap()
    }

    /// The `muster __service` command [`Self::spawn_service`] runs.
    fn service_command(&self) -> Command {
        let mut command = Command::new(BINARY);
        command
            .arg(SERVICE_SUBCOMMAND)
            .env("XDG_STATE_HOME", self.state_root())
            .env("XDG_CONFIG_HOME", self.root.path().join("config"))
            .env("XDG_DATA_HOME", self.root.path().join("data"))
            .env(LEAKED_VAR, "leaked")
            .env(INHERITED_VAR, "leaked")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    }

    /// Starts `muster __service` over this scratch's XDG roots and waits until
    /// it accepts.
    fn start_service(&self) -> Child {
        self.accepting(self.spawn_service())
    }

    /// Starts `muster __service` with an open-files soft limit of `soft`, as
    /// a user's own `ulimit` would leave it, and waits until it accepts.
    fn start_service_with_open_files(&self, soft: libc::rlim_t) -> Child {
        let mut command = self.service_command();
        // SAFETY: the hook only calls `getrlimit` and `setrlimit`, which are
        // async-signal-safe and touch only the stack value they are given.
        unsafe {
            command.pre_exec(move || {
                let mut limits = open_files_limits();
                limits.rlim_cur = soft;
                if libc::setrlimit(libc::RLIMIT_NOFILE, &limits) == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        self.accepting(command.spawn().unwrap())
    }

    /// Waits until the service `child` accepts, and returns it.
    fn accepting(&self, child: Child) -> Child {
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
    fn status(&self, project: &ProjectRoot) -> ProjectStatus {
        let ServiceResponse::Status(mut statuses) = self.send(ServiceRequest::Ps {
            project: Some(project.clone()),
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
    signal(pid, libc::SIGKILL);
}

/// Sends `signal` to the process `pid`.
fn signal(pid: u32, signal: libc::c_int) {
    // SAFETY: `kill` takes plain integers and touches no memory.
    unsafe {
        libc::kill(pid as libc::pid_t, signal);
    }
}

/// Whether the process `pid` died and waits to be reaped, or is gone.
fn exited(pid: u32) -> bool {
    fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .is_none_or(|stat| {
            stat.rsplit_once(") ")
                .is_some_and(|(_, fields)| fields.starts_with(ZOMBIE_STATE))
        })
}

/// A child runs with the scrubbed baseline and the environment of the client
/// that started it, never a variable the service itself inherited, and its
/// policy restarts keep that client environment.
#[test]
fn a_child_runs_with_the_client_environment_and_none_of_the_services() {
    let mut scratch = Scratch::new();
    let project = scratch.project_with(&[process("api", SLEEPER, RestartPolicy::Always)]);
    let service = scratch.start_service();
    let first = scratch.up_running(&project).pid().unwrap().into_inner();

    let client_entry = format!("{CLIENT_VAR}={CLIENT_VALUE}");
    let leaked = |pid: u32| {
        environ(pid)
            .iter()
            .any(|entry| entry.starts_with(LEAKED_VAR) || entry.starts_with(INHERITED_VAR))
    };
    let env = environ(first);
    assert!(env.contains(&client_entry), "{env:?}");
    assert!(!leaked(first), "{env:?}");
    assert!(
        env.contains(&format!("MUSTER_PROJECT={}", project.as_ref().display())),
        "{env:?}"
    );

    kill_hard(first);
    wait_until("the policy restart", || {
        scratch
            .api(&project)
            .pid()
            .is_some_and(|pid| pid.into_inner() != first)
    });
    let second = scratch.api(&project).pid().unwrap().into_inner();
    scratch.children.push(second);
    assert!(
        environ(second).contains(&client_entry),
        "kept for the restart"
    );
    assert!(!leaked(second));
    scratch.control(&project, ProcessAction::Stop);
    scratch.stop_service(service);
}

/// A child that outlives its SIGKILLed holder, as one that ignores the hangup
/// does, is left unsupervised: `start` never runs a second copy beside it,
/// neither under the service that saw the holder die nor under a successor.
#[test]
fn a_child_outliving_its_killed_holder_is_unsupervised_and_never_started_twice() {
    let mut scratch = Scratch::new();
    let project = scratch.project_with(&[process("api", HUP_IGNORING, RestartPolicy::Never)]);
    let service = scratch.start_service();
    let pid = scratch.up_running(&project).pid().map(Pid::into_inner);
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
    assert_eq!(scratch.api(&project).pid().map(Pid::into_inner), pid);

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
    let child = scratch.up_running(&project).pid().unwrap().into_inner();
    let holder = scratch.holders()[0];

    signal(holder, libc::SIGSTOP);
    kill_hard(child);
    wait_until("the child to die unreaped", || exited(child));
    kill_hard(holder);
    wait_until("the crash to be told", || {
        scratch
            .notices(&project)
            .iter()
            .any(|(_, message)| message.starts_with(CRASHED))
    });
    assert!(!scratch.api(&project).state().may_be_running());
    scratch.stop_service(service);
}

/// A process restarting under `restart: always` when the service stops keeps
/// restarting under the successor, which also keeps its restart count.
#[test]
fn a_restart_pending_at_service_stop_is_carried_out_by_the_successor() {
    let scratch = Scratch::new();
    let project = scratch.project_with(&[process("api", FLAPPER, RestartPolicy::Always)]);
    let service = scratch.start_service();
    scratch.up(&project);
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

/// A stack change reaches the running service: an added process waits
/// `Pending`, a changed running one keeps its pid, and a removed running one
/// is stopped and its holder ends.
#[test]
fn a_stack_change_adds_updates_and_stops_live() {
    let mut scratch = Scratch::new();
    let project = scratch.project_with(&[
        process("api", SLEEPER, RestartPolicy::Never),
        process("web", SLEEPER, RestartPolicy::Never),
    ]);
    let service = scratch.start_service();
    let api = scratch.up_running(&project).pid().unwrap();
    wait_until("web to run", || {
        scratch
            .named(&project, "web")
            .is_some_and(|web| *web.state() == ProcessState::Running)
    });
    let store = scratch.store();

    store
        .add_process(&project, &process("docs", SLEEPER, RestartPolicy::Never))
        .unwrap();
    store
        .remove_process(&project, &ProcessName::try_new("web").unwrap())
        .unwrap();
    assert_eq!(
        scratch.send(ServiceRequest::StackChanged {
            project: project.clone()
        }),
        ServiceResponse::Ok
    );

    assert_eq!(
        *scratch.named(&project, "docs").unwrap().state(),
        ProcessState::Pending
    );
    assert_eq!(*scratch.api(&project).pid(), Some(api), "api kept running");
    wait_until("web to be stopped and gone", || {
        scratch.named(&project, "web").is_none() && scratch.holders().len() == 1
    });
    scratch.stop_service(service);
}

/// A process removed from the project while no service ran is found running
/// by the next service, which stops it with a notice.
#[test]
fn a_process_removed_while_the_service_was_down_is_stopped_at_start() {
    let mut scratch = Scratch::new();
    let project = scratch.project();
    let service = scratch.start_service();
    let pid = scratch.up_running(&project).pid().unwrap().into_inner();
    scratch.stop_service(service);

    scratch
        .store()
        .remove_process(&project, &ProcessName::try_new("api").unwrap())
        .unwrap();
    let successor = scratch.start_service();

    wait_until("the removed process to be stopped", || {
        !alive(pid) && scratch.holders().is_empty()
    });
    assert!(
        scratch
            .notices(&project)
            .contains(&(Some("api".to_string()), REMOVED_NOTICE.to_string()))
    );
    scratch.stop_service(successor);
}

#[test]
fn a_service_exits_leaving_its_holder_running_and_a_successor_adopts_it() {
    let mut scratch = Scratch::new();
    let project = scratch.project();
    let service = scratch.start_service();

    let ServiceResponse::Batch(up) = scratch.up(&project) else {
        panic!("up answers what it started");
    };
    assert_eq!(up.acted().len(), 1, "{up:?}");
    let mut api = scratch.api(&project);
    wait_until("the process to run", || {
        api = scratch.api(&project);
        *api.state() == ProcessState::Running
    });
    let pid = api.pid().unwrap().into_inner();
    scratch.children.push(pid);

    scratch.stop_service(service);

    assert!(!scratch.socket().exists(), "the socket is removed");
    assert!(alive(pid), "the process outlives the service");
    assert_eq!(scratch.holders().len(), 1);

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
        *scratch.api(&project).state() == ProcessState::Running
    });
    assert_eq!(scratch.api(&project).pid().map(Pid::into_inner), Some(pid));

    scratch.control(&project, ProcessAction::Kill);
    wait_until("the kill to settle", || {
        !scratch.api(&project).state().may_be_running()
    });
    scratch.stop_service(successor);
    wait_until("the holder to finish", || scratch.holders().is_empty());
    assert!(!alive(pid));
}

/// A child started after the service has sampled usage runs with the
/// open-files soft limit the service started with, not the hard limit
/// `sysinfo` raised the service's own to.
#[test]
fn a_child_keeps_the_open_files_limit_the_service_started_with() {
    let hard = open_files_limits().rlim_max;
    assert!(
        hard > SERVICE_OPEN_FILES,
        "the hard limit leaves room to raise"
    );
    let mut scratch = Scratch::new();
    let project = scratch.project_with(&[
        process("api", SLEEPER, RestartPolicy::Never),
        process("probe", SLEEPER, RestartPolicy::Never).with_autostart(false),
    ]);
    let service = scratch.start_service_with_open_files(SERVICE_OPEN_FILES);
    scratch.up_running(&project);
    wait_until("a usage sample", || scratch.api(&project).usage().is_some());

    scratch.send(ServiceRequest::Control {
        project: project.clone(),
        name: ProcessName::try_new("probe").unwrap(),
        action: ProcessAction::Start,
        environment: client_env(),
        wait: None,
    });
    let mut probe = None;
    wait_until("the probe to run", || {
        probe = scratch
            .named(&project, "probe")
            .and_then(|probe| probe.pid().map(Pid::into_inner));
        probe.is_some()
    });
    let probe = probe.unwrap();
    scratch.children.push(probe);

    assert_eq!(soft_open_files(probe), SERVICE_OPEN_FILES.to_string());
    scratch.send(ServiceRequest::Down {
        project: project.clone(),
    });
    scratch.stop_service(service);
}

/// A stack write the service was never told about still reaches it: the next
/// `ps` shows the process it added and stops the one it removed.
#[test]
fn a_stack_write_whose_notice_was_missed_reaches_the_next_request() {
    let mut scratch = Scratch::new();
    let project = scratch.project();
    let service = scratch.start_service();
    scratch.up_running(&project);
    let store = scratch.store();

    store
        .add_process(
            &project,
            &process("docs", SLEEPER, RestartPolicy::Never).with_autostart(false),
        )
        .unwrap();
    store
        .remove_process(&project, &ProcessName::try_new("api").unwrap())
        .unwrap();

    assert_eq!(
        *scratch.named(&project, "docs").unwrap().state(),
        ProcessState::Pending
    );
    wait_until("api to be stopped and gone", || {
        scratch.named(&project, "api").is_none() && scratch.holders().is_empty()
    });
    scratch.stop_service(service);
}
