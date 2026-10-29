//! The CLI end to end: the built `muster` binary run as a user would, over
//! isolated XDG roots with desktop notifications off, starting its own
//! service on demand, replacing an older service, refusing a newer one, and
//! reporting what happened to processes.

#![cfg(target_os = "linux")]

use std::{
    fs,
    io::{self, BufRead, BufReader, Read, Write},
    os::{
        fd::OwnedFd,
        unix::{
            fs::OpenOptionsExt,
            net::{UnixListener, UnixStream},
            process::CommandExt,
        },
    },
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    sync::{Arc, Mutex, PoisonError},
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime},
};

use crossbeam_channel::{Receiver, Sender, unbounded};
use muster::{
    adapter::{
        config::YamlProjectRegistry,
        holder::{
            ClientRole, ExecHolderLauncher, HOLDER_SUBCOMMAND, HolderConfig, HolderDirs,
            HolderLauncher, HolderRunner, client::greet,
        },
        service::{
            BatchOutcome, Notice, NoticeLevel, ProcessAction, ProcessStatus, ProjectKey,
            ProjectPorts, ProjectStatus, Refusal, ServiceDeps, ServiceError, ServiceEvent,
            ServiceRequest, ServiceResponse, ServiceRuntime, Singleton,
            protocol::{self, SERVICE_SUBCOMMAND},
        },
    },
    domain::{
        color_scheme::ColorScheme,
        config::{ConfigError, WorkspaceConfig},
        port::{AgentSessionStore, ConfigWatcher, Notifier, ProcessMetrics, ProjectRegistry},
        process::{ActivityState, ProcessKind, ProcessState},
        project::Project,
        pty::{PtyError, PtySize},
        settings::Settings,
        value::{ByteSize, Cols, DetachKey, ProcessName, Rows},
    },
};
use rustix::{
    pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt},
    termios::{LocalModes, Winsize, tcgetattr, tcsetwinsize},
};
use serde_json::Value;
use tempfile::TempDir;

/// The muster binary cargo built for this test.
const BINARY: &str = env!("CARGO_BIN_EXE_muster");
/// The settings muster ships, which every scratch starts from.
const SHIPPED_SETTINGS: &str = include_str!("../src/adapter/config/default_settings.yml");
/// The shipped desktop notification setting, and what scratches use instead.
const DESKTOP_ON: &str = "desktop_notifications: true";
const DESKTOP_OFF: &str = "desktop_notifications: false";
/// Bound on every wait; reaching it fails the test.
const WAIT: Duration = Duration::from_secs(30);
/// Interval between checks of a polled condition.
const POLL: Duration = Duration::from_millis(20);
/// Short root for test sockets, inside `sun_path`.
const SHORT_TEMP_ROOT: &str = "/tmp";
/// Prefix of the scratch directory.
const SCRATCH_PREFIX: &str = "mc-";
/// Directory muster keeps its state and config under, inside the XDG roots.
const APP_DIR: &str = "muster";
/// The settings file name inside muster's config directory.
const SETTINGS_FILE: &str = "settings.yml";
/// Exit code of an action that failed.
const ACTION_FAILED: i32 = 1;
/// Exit code of a command line that does not parse.
const USAGE: i32 = 2;
/// Exit code of an unavailable or newer service.
const SERVICE_UNAVAILABLE: i32 = 3;
/// Exit code of an unknown name or project.
const INVALID_INPUT: i32 = 4;
/// Exit code of a command a person interrupted with Ctrl-C at a terminal.
const INTERRUPTED: i32 = 130;
/// How long the slow service takes to answer `ps`.
const SLOW_ANSWER: Duration = Duration::from_millis(1500);
/// How many commands race to start the service.
const RACERS: usize = 8;
/// The version an older in-process service reports.
const OLD_VERSION: &str = "0.0.1-old";
/// The version a newer in-process service reports.
const NEWER_VERSION: &str = "99.0.0";
/// The binary an in-process service reports running.
const IN_PROCESS_BINARY: &str = "/opt/other/muster";
/// How long the gated launcher holds a launch, so a replacement waits on it.
const GATE_HOLD: Duration = Duration::from_millis(1500);
/// What a service that lost the start race writes to its log.
const LOST_RACE_LINE: &str = "another muster service already runs";
/// A project of two long runners with a space and an accent in their names,
/// and a third that does not autostart.
const PROJECT: &str = "agents: []\nterminals: []\ncommands:\n  - name: dev server\n    command: echo dev-ready; sleep 600\n    autostart: true\n  - name: café\n    command: sleep 600\n    autostart: true\n  - name: api\n    command: echo api-ran; sleep 600\n    autostart: false\n";
/// A project whose second autostart process fails at once with status 3.
const FAILING_PROJECT: &str = "agents: []\nterminals: []\ncommands:\n  - name: web\n    command: sleep 600\n    autostart: true\n  - name: broken\n    command: echo oops; exit 3\n    autostart: true\n";
/// A project with one long runner.
const SINGLE_PROJECT: &str = "agents: []\nterminals: []\ncommands:\n  - name: api\n    command: sleep 600\n    autostart: true\n";
/// A project with a runner that takes a second to stop and a neighbour that
/// crash-loops meanwhile.
const CRASH_LOOP_PROJECT: &str = "agents: []\nterminals: []\ncommands:\n  - name: api\n    command: trap 'sleep 1; exit 0' TERM; while :; do sleep 0.1; done\n    autostart: true\n  - name: flaky\n    command: sleep 0.1; exit 1\n    autostart: true\n    restart: always\n";
/// A project with a process that ignores the stop signal for its grace.
const SLOW_STOP_PROJECT: &str = "agents: []\nterminals: []\ncommands:\n  - name: slow\n    command: trap '' TERM; while :; do sleep 0.1; done\n    autostart: true\n    stop:\n      signal: terminate\n      grace_period: 1s\n";
/// Why the scripted service leaves `api` alone on `up`.
const REFUSAL_REASON: &str = "cannot start: an earlier run may still be running as pid 4242";
/// A project with a process that writes a line every tenth of a second.
const TICKING_PROJECT: &str = "agents: []\nterminals: []\ncommands:\n  - name: ticker\n    command: while :; do echo tick; sleep 0.1; done\n    autostart: true\n";

/// Polls `condition` until it holds, failing at [`WAIT`].
fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let started = Instant::now();
    while !condition() {
        assert!(started.elapsed() < WAIT, "timed out waiting for {what}");
        thread::sleep(POLL);
    }
}

/// Pids of processes running `subcommand` whose command line or environment
/// mentions `root`.
fn processes_under(subcommand: &str, root: &Path) -> Vec<u32> {
    let root = root.to_string_lossy().into_owned();
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .filter(|pid| alive(*pid))
        .filter(|pid| {
            let cmdline = fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
            let environ = fs::read(format!("/proc/{pid}/environ")).unwrap_or_default();
            let cmdline = String::from_utf8_lossy(&cmdline);
            cmdline.contains(subcommand)
                && (cmdline.contains(&root) || String::from_utf8_lossy(&environ).contains(&root))
        })
        .collect()
}

/// Whether the process `pid` exists and is not a zombie.
fn alive(pid: u32) -> bool {
    fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
        !stat
            .rsplit_once(") ")
            .is_some_and(|(_, rest)| rest.starts_with('Z'))
    })
}

/// Isolated XDG roots and projects; dropping it kills every service, holder
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
        let scratch = Self {
            root,
            children: Vec::new(),
        };
        scratch.write_settings(&SHIPPED_SETTINGS.replace(DESKTOP_ON, DESKTOP_OFF));
        scratch
    }

    /// Replaces the settings file with `settings`.
    fn write_settings(&self, settings: &str) {
        let path = self.settings_path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, settings).unwrap();
    }

    /// The settings file every service over this scratch reads.
    fn settings_path(&self) -> PathBuf {
        self.root
            .path()
            .join("config")
            .join(APP_DIR)
            .join(SETTINGS_FILE)
    }

    /// The state directory every service over this scratch owns.
    fn state_dir(&self) -> PathBuf {
        self.root.path().join("state").join(APP_DIR)
    }

    /// Writes `config` as the `muster.yml` of the project in `folder`.
    fn project(&self, folder: &str, config: &str) -> PathBuf {
        let dir = self.root.path().join(folder);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("muster.yml");
        fs::write(&path, config).unwrap();
        path
    }

    /// `muster <args>` over this scratch's XDG roots, run from its root.
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(BINARY);
        command
            .args(args)
            .current_dir(self.root.path())
            .env("XDG_STATE_HOME", self.root.path().join("state"))
            .env("XDG_CONFIG_HOME", self.root.path().join("config"))
            .env("XDG_DATA_HOME", self.root.path().join("data"))
            .env_remove("MUSTER_PROJECT");
        command
    }

    /// Runs `muster <args>` and returns its output.
    fn muster(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    /// Runs `muster <args>`, with `--json` appended, and parses what it
    /// printed.
    fn json(&self, args: &[&str]) -> (Output, Value) {
        let mut all = args.to_vec();
        all.push("--json");
        let output = self.muster(&all);
        (output.clone(), parsed(&output))
    }

    /// The `ps` row of process `name` in `project`.
    fn row(&self, project: &Path, name: &str) -> Value {
        let (_, statuses) = self.json(&["-c", project.to_str().unwrap(), "ps"]);
        statuses[0]["processes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|process| process["name"] == name)
            .cloned()
            .unwrap_or_else(|| panic!("no row {name}: {statuses}"))
    }

    /// Sends `request` straight to the service socket, as an in-process
    /// client.
    fn send(&self, request: &ServiceRequest) -> ServiceResponse {
        protocol::send(&protocol::socket_path(&self.state_dir()), request).unwrap()
    }

    /// The state and pid of the first process of `project`, as the service
    /// lists it.
    fn first_row_raw(&self, project: &Path) -> (String, Option<u64>) {
        let response = self.send(&ServiceRequest::Ps {
            project: Some(project.to_path_buf()),
        });
        let value = serde_json::to_value(response).unwrap();
        let row = &value["status"][0]["processes"][0];
        (
            row["state"].as_str().unwrap().to_string(),
            row["pid"].as_u64(),
        )
    }

    /// What the services over this scratch wrote to their log.
    fn service_log(&self) -> String {
        fs::read_to_string(protocol::log_path(&self.state_dir())).unwrap_or_default()
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

/// What `output` printed on stdout, parsed as JSON.
fn parsed(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!("printed no JSON ({error}): {}", shown(output));
    })
}

/// Prints a command's output when a check on it fails.
fn shown(output: &Output) -> String {
    format!(
        "status {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

/// Every test port of a service run in the test process, reading project
/// configs straight from their files and nothing of the user's.
struct FileOnlyPorts;

/// A registry that reads a project's config from its own path and knows no
/// registered projects.
struct FileRegistry;

impl ProjectRegistry for FileRegistry {
    fn projects(&self) -> Result<Vec<Project>, ConfigError> {
        Ok(Vec::new())
    }

    fn workspace(&self, config_path: &Path) -> Result<WorkspaceConfig, ConfigError> {
        YamlProjectRegistry.workspace(config_path)
    }

    fn workspace_exists(&self, config_path: &Path) -> bool {
        config_path.exists()
    }

    fn save(&self, _projects: &[Project]) -> Result<(), ConfigError> {
        Ok(())
    }

    fn save_workspace(&self, _path: &Path, _config: &WorkspaceConfig) -> Result<(), ConfigError> {
        Ok(())
    }
}

impl ProjectPorts for FileOnlyPorts {
    fn registry(&self) -> Box<dyn ProjectRegistry> {
        Box::new(FileRegistry)
    }

    fn agent_session_store(&self) -> Option<Box<dyn AgentSessionStore>> {
        None
    }

    fn notifier(&self) -> Option<Box<dyn Notifier>> {
        None
    }

    fn process_metrics(&self) -> Option<Box<dyn ProcessMetrics>> {
        None
    }

    fn config_watcher(&self, _events: Sender<ServiceEvent>) -> Option<Box<dyn ConfigWatcher>> {
        None
    }
}

/// Launches real holders from the built binary, but only after holding each
/// launch for [`GATE_HOLD`], so a launch is in flight when the service is
/// asked to make way.
struct HeldLauncher(ExecHolderLauncher);

impl HolderLauncher for HeldLauncher {
    fn launch(&self, config: HolderConfig) -> Result<(), PtyError> {
        thread::sleep(GATE_HOLD);
        self.0.launch(config)
    }
}

/// Settings with every value explicit and desktop notifications off.
fn settings() -> Settings {
    Settings::builder()
        .desktop_notifications(false)
        .default_size(
            PtySize::builder()
                .rows(Rows::new(24))
                .cols(Cols::new(80))
                .build(),
        )
        .detach_key(DetachKey::try_new("ctrl-\\").unwrap())
        .log_max_size(ByteSize::try_new(bytesize::ByteSize::mib(4)).unwrap())
        .scrollback(1000)
        .color_scheme(ColorScheme::Dark)
        .build()
}

/// Runs a service reporting `version` in this process over `state_dir`,
/// launching real holders from the built binary, held by [`GATE_HOLD`] when
/// `held` is set.
fn in_process_service(
    state_dir: PathBuf,
    version: &str,
    held: bool,
) -> JoinHandle<Result<(), ServiceError>> {
    let started = state_dir.clone();
    let version = version.to_string();
    let service = thread::spawn(move || {
        let singleton = Singleton::acquire(&state_dir)?.expect("no other service runs");
        let exec = ExecHolderLauncher::builder()
            .binary(PathBuf::from(BINARY))
            .build();
        let launcher: Box<dyn HolderLauncher> = if held {
            Box::new(HeldLauncher(exec))
        } else {
            Box::new(exec)
        };
        let settings = settings();
        let runner = HolderRunner::new(
            launcher,
            HolderDirs::under(&state_dir),
            *settings.log_max_size(),
            *settings.scrollback(),
            *settings.color_scheme(),
        );
        let deps = ServiceDeps::builder()
            .version(version)
            .binary(PathBuf::from(IN_PROCESS_BINARY))
            .holders(Arc::new(runner))
            .ports(Box::new(FileOnlyPorts))
            .settings(settings)
            .build();
        ServiceRuntime::run(singleton, deps)
    });
    wait_until("the in-process service to accept", || {
        UnixStream::connect(protocol::socket_path(&started)).is_ok()
    });
    service
}

/// Review Focus 2: commands racing against an empty state directory start
/// exactly one service between them, and every one of them succeeds.
#[test]
fn concurrent_connects_start_exactly_one_service() {
    let scratch = Arc::new(Scratch::new());
    let project = scratch.project("shop", SINGLE_PROJECT);

    let racers: Vec<JoinHandle<Output>> = (0..RACERS)
        .map(|_| {
            let scratch = Arc::clone(&scratch);
            let project = project.clone();
            thread::spawn(move || {
                scratch.muster(&["-c", project.to_str().unwrap(), "ps", "--json"])
            })
        })
        .collect();
    for racer in racers {
        let output = racer.join().unwrap();
        assert!(output.status.success(), "{}", shown(&output));
    }

    let services = processes_under(SERVICE_SUBCOMMAND, scratch.root.path());
    assert_eq!(services.len(), 1, "one service runs: {services:?}");
    assert!(
        !scratch.service_log().contains(LOST_RACE_LINE),
        "no racer even spawned a second service: {}",
        scratch.service_log()
    );
    let (_, status) = scratch.json(&["service", "status"]);
    assert_eq!(status["running"], true);
    assert_eq!(status["version"], status["client_version"]);
}

/// A command from a newer binary replaces an older service, whose
/// processes keep running and are adopted by the new service.
#[test]
fn version_mismatch_replaces_the_service_and_keeps_holders() {
    let mut scratch = Scratch::new();
    let project = scratch.project("shop", SINGLE_PROJECT);
    let config = project.to_str().unwrap().to_string();
    let old = in_process_service(scratch.state_dir(), OLD_VERSION, false);
    assert!(matches!(
        scratch.send(&ServiceRequest::Up {
            project: project.clone(),
        }),
        ServiceResponse::Batch(_)
    ));
    let mut pid = None;
    wait_until("the old service to run api", || {
        let (state, running) = scratch.first_row_raw(&project);
        pid = running.filter(|_| state == "running");
        pid.is_some()
    });
    let pid = pid.unwrap() as u32;
    scratch.children.push(pid);

    let (output, status) = scratch.json(&["service", "status"]);
    assert!(output.status.success(), "{}", shown(&output));
    assert_eq!(status["version"], OLD_VERSION, "status never replaces");

    let ps = scratch.muster(&["-c", &config, "ps"]);
    assert!(ps.status.success(), "{}", shown(&ps));
    wait_until("the old service to make way", || old.is_finished());
    assert!(
        old.join().unwrap().is_ok(),
        "the old service exited cleanly when asked to make way"
    );

    let (_, status) = scratch.json(&["service", "status"]);
    assert_eq!(status["version"], status["client_version"]);
    assert_eq!(
        processes_under(SERVICE_SUBCOMMAND, scratch.root.path()).len(),
        1
    );
    assert!(alive(pid), "the process outlived the replacement");
    wait_until("the new service to adopt api", || {
        let row = scratch.row(&project, "api");
        row["state"] == "running" && row["pid"] == pid
    });
    assert_eq!(
        processes_under(HOLDER_SUBCOMMAND, scratch.root.path()).len(),
        1
    );
}

/// A second command arriving while an older service waits out a launch
/// before making way does not spawn a stream of services: it spawns at most
/// one, which exits at once, and then reaches the replacement.
#[test]
fn a_command_during_a_slow_replacement_reaches_the_new_service() {
    let mut scratch = Scratch::new();
    let project = scratch.project("shop", SINGLE_PROJECT);
    let config = project.to_str().unwrap().to_string();
    let old = in_process_service(scratch.state_dir(), OLD_VERSION, true);
    let mut pid = None;
    scratch.send(&ServiceRequest::Up {
        project: project.clone(),
    });
    wait_until("the held launch to run api", || {
        let (state, running) = scratch.first_row_raw(&project);
        pid = running.filter(|_| state == "running");
        pid.is_some()
    });
    scratch.children.push(pid.unwrap() as u32);
    scratch.send(&ServiceRequest::Control {
        project: project.clone(),
        name: ProcessName::try_new("api").unwrap(),
        action: ProcessAction::Restart,
    });
    wait_until("the restart's launch to be held", || {
        scratch.first_row_raw(&project).0 == "starting"
    });

    let mut replacing: Child = scratch
        .command(&["-c", &config, "ps"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_until("the old service to start making way", || {
        !protocol::socket_path(&scratch.state_dir()).exists()
            || replacing.try_wait().unwrap().is_some()
    });
    let second = scratch.muster(&["-c", &config, "ps", "--json"]);

    assert!(second.status.success(), "{}", shown(&second));
    let replacing = replacing.wait_with_output().unwrap();
    assert!(replacing.status.success());
    wait_until("the old service to finish", || old.is_finished());
    assert!(
        scratch.service_log().matches(LOST_RACE_LINE).count() <= 1,
        "at most one extra service was spawned: {}",
        scratch.service_log()
    );
    assert_eq!(
        processes_under(SERVICE_SUBCOMMAND, scratch.root.path()).len(),
        1
    );
}

/// An older command meeting a newer service refuses, naming both versions
/// and the binary the service runs, and leaves the service alone.
#[test]
fn an_older_client_refuses_a_newer_service() {
    let scratch = Scratch::new();
    let project = scratch.project("shop", SINGLE_PROJECT);
    let newer = in_process_service(scratch.state_dir(), NEWER_VERSION, false);

    let ps = scratch.muster(&["-c", project.to_str().unwrap(), "ps"]);

    assert_eq!(
        ps.status.code(),
        Some(SERVICE_UNAVAILABLE),
        "{}",
        shown(&ps)
    );
    let stderr = String::from_utf8_lossy(&ps.stderr);
    assert!(
        stderr.contains(NEWER_VERSION)
            && stderr.contains(env!("CARGO_PKG_VERSION"))
            && stderr.contains(IN_PROCESS_BINARY),
        "{stderr}"
    );
    assert!(!newer.is_finished(), "the newer service keeps running");
    assert_eq!(scratch.send(&ServiceRequest::Stop), ServiceResponse::Ok);
    wait_until("the newer service to stop", || newer.is_finished());
}

/// Review Focus 4 from the command line: names with a space or an accent,
/// typed in either Unicode form, address their process; an unknown name
/// lists every valid one and exits as invalid input.
#[test]
fn names_with_spaces_and_unicode_resolve_from_the_command_line() {
    let scratch = Scratch::new();
    let project = scratch.project("shop", PROJECT);
    let config = project.to_str().unwrap();

    let (output, report) = scratch.json(&["-c", config, "start", "dev server"]);
    assert!(output.status.success(), "{}", shown(&output));
    assert_eq!(report["processes"][0]["name"], "dev server");
    assert_eq!(report["processes"][0]["outcome"], "started");

    let (output, report) = scratch.json(&["-c", config, "start", "cafe\u{301}"]);
    assert!(output.status.success(), "{}", shown(&output));
    assert_eq!(report["processes"][0]["name"], "caf\u{e9}");

    let unknown = scratch.muster(&["-c", config, "stop", "dev"]);
    assert_eq!(unknown.status.code(), Some(INVALID_INPUT));
    let stderr = String::from_utf8_lossy(&unknown.stderr);
    assert!(
        stderr.contains("no process 'dev' in")
            && stderr.contains("valid names: 'dev server', ")
            && stderr.contains("api"),
        "{stderr}"
    );
    let (unknown, failure) = scratch.json(&["-c", config, "stop", "dev"]);
    assert_eq!(unknown.status.code(), Some(INVALID_INPUT));
    assert_eq!(failure["error"]["kind"], "unknown-process");
    assert_eq!(
        failure["error"]["candidates"],
        serde_json::json!(["dev server", "café", "api"])
    );

    let (output, report) = scratch.json(&["-c", config, "stop", "dev server"]);
    assert!(output.status.success(), "{}", shown(&output));
    assert_eq!(report["processes"][0]["outcome"], "stopped");
}

/// `up` reports what the service started, `ps` lists them, `logs` prints
/// what they wrote, `down` stops them; a crash is reported with its exit,
/// fails the command, and its log stays readable after it exited.
#[test]
fn up_ps_logs_and_down_report_what_happened() {
    let scratch = Scratch::new();
    let project = scratch.project("shop", PROJECT);
    let config = project.to_str().unwrap();

    let up = scratch.muster(&["-c", config, "up"]);
    assert!(up.status.success(), "{}", shown(&up));
    let text = String::from_utf8_lossy(&up.stdout);
    assert!(
        text.contains("dev server") && text.contains("running (pid"),
        "{text}"
    );
    assert!(!text.contains("api"), "api does not autostart: {text}");

    let again = scratch.muster(&["-c", config, "up"]);
    assert!(
        String::from_utf8_lossy(&again.stdout).contains("nothing to start"),
        "{}",
        shown(&again)
    );

    let stable = |output: &Output| -> Vec<String> {
        let text = String::from_utf8_lossy(&output.stdout).into_owned();
        let header = text.lines().next().unwrap_or_default().to_string();
        let width = header.find("UPTIME").unwrap_or(header.len());
        text.lines()
            .map(|line| line.chars().take(width).collect::<String>())
            .collect()
    };
    let ps = scratch.muster(&["-c", config, "ps"]);
    assert!(
        String::from_utf8_lossy(&ps.stdout).starts_with("NAME"),
        "{}",
        shown(&ps)
    );
    assert_eq!(
        stable(&ps),
        stable(&scratch.muster(&["-c", config])),
        "bare muster prints ps"
    );

    wait_until("the dev server to write", || {
        String::from_utf8_lossy(&scratch.muster(&["-c", config, "logs", "dev server"]).stdout)
            .contains("dev-ready")
    });

    let (down, report) = scratch.json(&["-c", config, "down"]);
    assert!(down.status.success(), "{}", shown(&down));
    let outcomes: Vec<&str> = report["processes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|process| process["outcome"].as_str())
        .collect();
    assert_eq!(outcomes, vec!["stopped", "stopped"], "{report}");
    assert!(
        String::from_utf8_lossy(&scratch.muster(&["-c", config, "logs", "dev server"]).stdout)
            .contains("dev-ready"),
        "a stopped process's log is still read"
    );

    let failing = scratch.project("broken", FAILING_PROJECT);
    let failing = failing.to_str().unwrap();
    let up = scratch.muster(&["-c", failing, "up"]);
    assert_eq!(up.status.code(), Some(ACTION_FAILED), "{}", shown(&up));
    let text = String::from_utf8_lossy(&up.stdout);
    assert!(
        text.contains("broken") && text.contains("crashed: exited with status 3"),
        "{text}"
    );
    assert!(text.contains("web") && text.contains("running"), "{text}");
    let broken = scratch.row(Path::new(failing), "broken");
    assert_eq!(broken["state"], "crashed");
    assert_eq!(broken["last_exit"]["status"], 3);
    assert!(
        String::from_utf8_lossy(&scratch.muster(&["-c", failing, "logs", "broken"]).stdout)
            .contains("oops"),
        "a crashed process's log is read after the fact"
    );
    let (_, notices) = scratch.json(&["-c", failing, "notices"]);
    assert!(
        notices.as_array().unwrap().iter().any(|notice| {
            notice["process"] == "broken" && notice["message"] == "crashed: exited with status 3"
        }),
        "{notices}"
    );
}

/// A neighbour crash-looping while the command runs is context, not a
/// reason for the command to fail.
#[test]
fn a_crash_looping_neighbour_does_not_fail_a_stop() {
    let scratch = Scratch::new();
    let project = scratch.project("shop", CRASH_LOOP_PROJECT);
    let config = project.to_str().unwrap();
    scratch.muster(&["-c", config, "up"]);
    wait_until("api to run", || {
        scratch.row(&project, "api")["state"] == "running"
    });

    let (stop, report) = scratch.json(&["-c", config, "stop", "api"]);

    assert!(stop.status.success(), "{}", shown(&stop));
    assert_eq!(report["processes"][0]["outcome"], "stopped");
    assert!(report["notices"].as_array().unwrap().is_empty(), "{report}");
    assert!(
        report["other_notices"]
            .as_array()
            .unwrap()
            .iter()
            .any(|notice| notice["process"] == "flaky" && notice["level"] == "error"),
        "the neighbour crashed during the stop and is shown as context: {report}"
    );
    scratch.muster(&["-c", config, "stop", "flaky"]);
}

/// A stop waits out the process's configured grace and the kill after it,
/// and reports the process stopped.
#[test]
fn a_stop_waits_for_a_slow_process() {
    let scratch = Scratch::new();
    let project = scratch.project("shop", SLOW_STOP_PROJECT);
    let config = project.to_str().unwrap();
    scratch.muster(&["-c", config, "up"]);
    wait_until("slow to run", || {
        scratch.row(&project, "slow")["state"] == "running"
    });

    let stop = scratch.muster(&["-c", config, "stop", "slow"]);

    assert!(stop.status.success(), "{}", shown(&stop));
    assert!(
        String::from_utf8_lossy(&stop.stdout).contains("slow: stopped"),
        "{}",
        shown(&stop)
    );
    assert_eq!(scratch.row(&project, "slow")["stop_grace"], 1);
}

/// `logs -f` of a stopped process shows its log and then follows it when it
/// starts again; a reader that goes away ends it quietly.
#[test]
fn logs_follow_waits_for_a_stopped_process_and_a_closed_pipe_ends_it() {
    let scratch = Scratch::new();
    let project = scratch.project("shop", PROJECT);
    let config = project.to_str().unwrap();

    let mut follow = scratch
        .command(&["-c", config, "logs", "-f", "api"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let (lines, received) = unbounded();
    let mut stdout = BufReader::new(follow.stdout.take().unwrap());
    let reader = thread::spawn(move || {
        let mut line = String::new();
        while stdout.read_line(&mut line).is_ok_and(|read| read > 0) {
            let _ = lines.send(line.clone());
            line.clear();
        }
    });
    let start = scratch.muster(&["-c", config, "start", "api"]);
    assert!(start.status.success(), "{}", shown(&start));
    let got = received.recv_timeout(WAIT).unwrap();
    assert!(got.contains("api-ran"), "{got}");
    let _ = follow.kill();
    let _ = follow.wait();
    reader.join().unwrap();

    let ticking = scratch.project("tick", TICKING_PROJECT);
    scratch.muster(&["-c", ticking.to_str().unwrap(), "up"]);
    let mut follow = scratch
        .command(&["-c", ticking.to_str().unwrap(), "logs", "-f", "ticker"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut first = [0u8; 1];
    follow
        .stdout
        .as_mut()
        .unwrap()
        .read_exact(&mut first)
        .unwrap();
    drop(follow.stdout.take());
    let mut ended = None;
    wait_until("the follow to notice the closed pipe", || {
        ended = follow.try_wait().unwrap();
        ended.is_some()
    });
    let mut stderr = String::new();
    follow
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut stderr)
        .unwrap();
    assert_eq!(ended.unwrap().code(), Some(0), "{stderr}");
    assert!(stderr.is_empty(), "{stderr}");
}

/// `muster run` adds the command to the project and starts it under the
/// service; bad arguments fail before anything is written or started.
#[test]
fn run_registers_the_command_and_starts_it_under_the_service() {
    let scratch = Scratch::new();
    let project = scratch.project("shop", SINGLE_PROJECT);
    let config = project.to_str().unwrap();

    let blank = scratch.muster(&["-c", config, "run", "--name", " ", "--", " "]);
    assert_eq!(
        blank.status.code(),
        Some(INVALID_INPUT),
        "{}",
        shown(&blank)
    );
    assert!(
        processes_under(SERVICE_SUBCOMMAND, scratch.root.path()).is_empty(),
        "a bad command line starts no service"
    );

    let output = scratch.muster(&[
        "-c", config, "run", "--json", "--name", "tail job", "--", "sleep", "600",
    ]);
    assert!(output.status.success(), "{}", shown(&output));
    let report = parsed(&output);
    assert_eq!(report["action"], "run");
    assert_eq!(report["processes"][0]["outcome"], "started");
    assert!(fs::read_to_string(&project).unwrap().contains("tail job"));
    assert_eq!(scratch.row(&project, "tail job")["state"], "running");

    let first = scratch.muster(&["-c", config, "run", "--", "sleep", "600"]);
    assert!(first.status.success(), "{}", shown(&first));
    let before = fs::read_to_string(&project).unwrap();
    let second = scratch.muster(&["-c", config, "run", "--", "sleep", "601"]);
    assert_eq!(
        second.status.code(),
        Some(INVALID_INPUT),
        "{}",
        shown(&second)
    );
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(
        stderr.contains("a process named 'sleep' already exists in")
            && stderr.contains("pass --name to choose another"),
        "the default name is refused with a hint: {stderr}"
    );
    assert_eq!(
        fs::read_to_string(&project).unwrap(),
        before,
        "nothing was written"
    );
}

/// `muster run` refuses a name an open agent session holds though the config
/// does not list it, and writes nothing.
#[test]
fn run_refuses_a_name_a_session_holds() {
    let scratch = Scratch::new();
    let project = scratch.project("shop", "agents: []\nterminals: []\ncommands: []\n");
    let config = project.to_str().unwrap();
    let before = fs::read_to_string(&project).unwrap();
    scripted_service(&scratch, project.clone());

    let run = scratch.muster(&["-c", config, "run", "--name", "api", "--", "sleep", "600"]);

    assert_eq!(run.status.code(), Some(INVALID_INPUT), "{}", shown(&run));
    assert!(
        String::from_utf8_lossy(&run.stderr).contains("a process named 'api' already exists in"),
        "{}",
        shown(&run)
    );
    assert_eq!(fs::read_to_string(&project).unwrap(), before);
}

/// `service stop` leaves every process running; the next command brings a
/// service back that adopts them.
#[test]
fn service_stop_keeps_processes_and_the_next_command_restarts_it() {
    let scratch = Scratch::new();
    let project = scratch.project("shop", SINGLE_PROJECT);
    let config = project.to_str().unwrap();
    scratch.muster(&["-c", config, "up"]);
    let pid = scratch.row(&project, "api")["pid"].as_u64().unwrap() as u32;

    let (stop, stopped) = scratch.json(&["service", "stop"]);
    assert!(stop.status.success(), "{}", shown(&stop));
    assert_eq!(stopped, serde_json::json!({ "stopped": true }));
    let (_, status) = scratch.json(&["service", "status"]);
    assert_eq!(status["running"], false);
    assert!(alive(pid));

    wait_until("the restarted service to adopt api", || {
        let row = scratch.row(&project, "api");
        row["state"] == "running" && row["pid"] == pid
    });
    let (stop, stopped) = scratch.json(&["service", "stop"]);
    assert!(stop.status.success());
    assert_eq!(stopped["stopped"], true);
    let (_, stopped) = scratch.json(&["service", "stop"]);
    assert_eq!(
        stopped["stopped"], false,
        "nothing was running the second time"
    );
}

/// A service that cannot start, here for a settings file missing a key,
/// makes the command fail with the reason from this start's log lines only.
#[test]
fn a_service_that_cannot_start_shows_why() {
    let scratch = Scratch::new();
    let project = scratch.project("shop", SINGLE_PROJECT);
    fs::create_dir_all(scratch.state_dir()).unwrap();
    fs::write(
        protocol::log_path(&scratch.state_dir()),
        "an older failure from another day\n",
    )
    .unwrap();
    scratch.write_settings(&format!("{DESKTOP_OFF}\n"));

    let ps = scratch.muster(&["-c", project.to_str().unwrap(), "ps"]);

    assert_eq!(ps.status.code(), Some(SERVICE_UNAVAILABLE));
    let stderr = String::from_utf8_lossy(&ps.stderr);
    assert!(
        stderr.contains("the muster service is not available"),
        "{stderr}"
    );
    assert!(
        stderr.contains(&scratch.settings_path().display().to_string()),
        "{stderr}"
    );
    assert!(
        !stderr.contains("older failure"),
        "only this start's lines: {stderr}"
    );
}

/// `project` listing one process, `api`, that has not started.
fn pending_api_status(project: &Path) -> ProjectStatus {
    ProjectStatus::builder()
        .project(ProjectKey::new(project.to_path_buf()))
        .processes(vec![
            ProcessStatus::builder()
                .name(ProcessName::try_new("api").unwrap())
                .kind(ProcessKind::Command)
                .state(ProcessState::Pending)
                .activity(ActivityState::Idle)
                .pid(None)
                .uptime(None)
                .restarts(0)
                .usage(None)
                .attached(0)
                .last_exit(None)
                .stop_grace(None)
                .build(),
        ])
        .build()
}

/// What a request asked the slow service, by name.
fn request_name(request: &ServiceRequest) -> &'static str {
    match request {
        ServiceRequest::Hello { .. } => "hello",
        ServiceRequest::Ps { .. } => "ps",
        ServiceRequest::Up { .. } => "up",
        _ => "other",
    }
}

/// A service on `scratch`'s socket that answers `ps` of `project` only
/// after [`SLOW_ANSWER`], so a command can be interrupted before it sends
/// what it asks for. Returns what it was asked, in order.
fn slow_status_service(scratch: &Scratch, project: PathBuf) -> Arc<Mutex<Vec<&'static str>>> {
    fs::create_dir_all(scratch.state_dir()).unwrap();
    let listener = UnixListener::bind(protocol::socket_path(&scratch.state_dir())).unwrap();
    let asked = Arc::new(Mutex::new(Vec::new()));
    let heard = Arc::clone(&asked);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else {
                return;
            };
            let mut line = String::new();
            if BufReader::new(&stream).read_line(&mut line).is_err() {
                continue;
            }
            let Ok(request) = serde_json::from_str::<ServiceRequest>(&line) else {
                continue;
            };
            heard
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(request_name(&request));
            let response = match request {
                ServiceRequest::Hello { .. } => ServiceResponse::Hello {
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    binary: PathBuf::from(IN_PROCESS_BINARY),
                },
                ServiceRequest::Ps { .. } => {
                    thread::sleep(SLOW_ANSWER);
                    ServiceResponse::Status(vec![pending_api_status(&project)])
                },
                _ => ServiceResponse::Ok,
            };
            let mut answer = serde_json::to_string(&response).unwrap();
            answer.push('\n');
            let _ = (&stream).write_all(answer.as_bytes());
        }
    });
    asked
}

/// Serves a scripted service on `scratch`'s socket for the rest of the test:
/// it answers hello as this version, lists `api` of `project` as pending,
/// answers `up` by leaving `api` alone with [`REFUSAL_REASON`], and reports
/// the matching error notice raised now.
fn scripted_service(scratch: &Scratch, project: PathBuf) {
    fs::create_dir_all(scratch.state_dir()).unwrap();
    let listener = UnixListener::bind(protocol::socket_path(&scratch.state_dir())).unwrap();
    let api = ProcessName::try_new("api").unwrap();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else {
                return;
            };
            let mut line = String::new();
            if BufReader::new(&stream).read_line(&mut line).is_err() {
                continue;
            }
            let Ok(request) = serde_json::from_str::<ServiceRequest>(&line) else {
                continue;
            };
            let response = match request {
                ServiceRequest::Hello { .. } => ServiceResponse::Hello {
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    binary: PathBuf::from(IN_PROCESS_BINARY),
                },
                ServiceRequest::Ps { .. } => {
                    ServiceResponse::Status(vec![pending_api_status(&project)])
                },
                ServiceRequest::Up { .. } => ServiceResponse::Batch(
                    BatchOutcome::builder()
                        .acted(vec![])
                        .refused(vec![
                            Refusal::builder()
                                .name(api.clone())
                                .reason(REFUSAL_REASON.to_string())
                                .build(),
                        ])
                        .build(),
                ),
                ServiceRequest::Notices { .. } => ServiceResponse::Notices(vec![
                    Notice::builder()
                        .at(SystemTime::now())
                        .project(Some(ProjectKey::new(project.clone())))
                        .process(Some(api.clone()))
                        .level(NoticeLevel::Error)
                        .message(REFUSAL_REASON.to_string())
                        .build(),
                ]),
                _ => ServiceResponse::Ok,
            };
            let mut answer = serde_json::to_string(&response).unwrap();
            answer.push('\n');
            let _ = (&stream).write_all(answer.as_bytes());
        }
    });
}

/// `up` reports a process it had to leave alone with the service's reason,
/// once, on that process's row, and counts the notice as its own.
#[test]
fn an_up_refusal_is_reported_once_with_its_reason() {
    let scratch = Scratch::new();
    let project = scratch.project("shop", SINGLE_PROJECT);
    let config = project.to_str().unwrap();
    let reason = REFUSAL_REASON.to_string();
    scripted_service(&scratch, project.clone());

    let (up, report) = scratch.json(&["-c", config, "up"]);
    let human = scratch.muster(&["-c", config, "up"]);

    assert_eq!(up.status.code(), Some(ACTION_FAILED), "{}", shown(&up));
    let refused = &report["processes"][0];
    assert_eq!(refused["outcome"], "refused", "{report}");
    assert_eq!(refused["reason"], reason.as_str());
    assert!(
        report["notices"]
            .as_array()
            .unwrap()
            .iter()
            .any(|notice| notice["message"] == reason.as_str()),
        "the refusal's notice is the report's own: {report}"
    );
    assert!(
        !report["other_notices"]
            .as_array()
            .unwrap()
            .iter()
            .any(|notice| notice["message"] == reason.as_str()),
        "{report}"
    );
    let text = String::from_utf8_lossy(&human.stdout);
    assert_eq!(text.matches(reason.as_str()).count(), 1, "{text}");
}

/// `service stop` sends `Stop` straight away, so it stops a service whose
/// hello this muster cannot read.
#[test]
fn service_stop_needs_no_hello() {
    let scratch = Scratch::new();
    fs::create_dir_all(scratch.state_dir()).unwrap();
    let listener = UnixListener::bind(protocol::socket_path(&scratch.state_dir())).unwrap();
    let service = thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut request = String::new();
        BufReader::new(&stream).read_line(&mut request).unwrap();
        let answer = if request.trim() == "\"stop\"" {
            "\"ok\"\n"
        } else {
            "{\"hello\":{\"from\":\"a muster of another shape\"}}\n"
        };
        (&stream).write_all(answer.as_bytes()).unwrap();
        request
    });

    let (stop, stopped) = scratch.json(&["service", "stop"]);

    assert!(stop.status.success(), "{}", shown(&stop));
    assert_eq!(stopped["stopped"], true);
    assert_eq!(service.join().unwrap().trim(), "\"stop\"");
}

/// Commands outside the service exit with the codes the help documents:
/// an invalid config and an unknown project are invalid input.
#[test]
fn check_and_projects_exit_with_the_documented_codes() {
    let scratch = Scratch::new();
    let duplicate = scratch.project(
        "dup",
        "agents: []\nterminals: []\ncommands:\n  - name: api\n    command: sleep 1\n    autostart: false\n  - name: api\n    command: sleep 2\n    autostart: false\n",
    );

    let check = scratch.muster(&["-c", duplicate.to_str().unwrap(), "check"]);
    assert_eq!(
        check.status.code(),
        Some(INVALID_INPUT),
        "{}",
        shown(&check)
    );
    assert!(
        String::from_utf8_lossy(&check.stdout).contains("command #1 and command #2"),
        "{}",
        shown(&check)
    );

    let remove = scratch.muster(&["projects", "remove", "nosuch"]);
    assert_eq!(
        remove.status.code(),
        Some(INVALID_INPUT),
        "{}",
        shown(&remove)
    );

    let ps = scratch.muster(&["-c", duplicate.to_str().unwrap(), "ps"]);
    assert_eq!(
        ps.status.code(),
        Some(INVALID_INPUT),
        "a project with a duplicated name does not load: {}",
        shown(&ps)
    );
}

/// A command outside any project fails before it starts a service, and a
/// usage error with `--json` is still JSON.
#[test]
fn input_errors_fail_before_the_service_starts() {
    let scratch = Scratch::new();

    let bare = scratch.muster(&[]);
    assert_eq!(bare.status.code(), Some(INVALID_INPUT), "{}", shown(&bare));
    assert!(
        String::from_utf8_lossy(&bare.stderr).contains("no muster config at"),
        "{}",
        shown(&bare)
    );
    assert!(
        processes_under(SERVICE_SUBCOMMAND, scratch.root.path()).is_empty(),
        "no service was started"
    );

    let usage = scratch.muster(&["start", "--bogus", "--json"]);
    assert_eq!(usage.status.code(), Some(USAGE));
    assert_eq!(parsed(&usage)["error"]["kind"], "usage");
}

/// The MCP protocol version the test client speaks.
const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
/// The JSON-RPC version every MCP message carries.
const JSON_RPC_VERSION: &str = "2.0";
/// A project for the MCP tools: a `cat` that echoes what it is sent, and a
/// runner with an accent in its name. Neither autostarts.
const MCP_PROJECT: &str = "agents: []\nterminals: []\ncommands:\n  - name: echo box\n    command: cat\n    autostart: false\n  - name: café\n    command: echo café-ready; sleep 600\n    autostart: false\n";
/// The tools that act on one process, each of which takes its `name`.
const PROCESS_TOOLS: [&str; 5] = [
    "process_output",
    "process_send",
    "process_start",
    "process_stop",
    "process_restart",
];

/// A `muster mcp` server run over stdio, spoken to as an agent would. Each
/// line it prints is a JSON message, or the raw line when it is not one.
struct McpSession {
    child: Child,
    lines: Receiver<Result<Value, String>>,
    next_id: u64,
}

impl McpSession {
    /// Starts `muster mcp` for `project` over `scratch` and initializes it.
    fn start(scratch: &mut Scratch, project: &Path) -> Self {
        let mut child = scratch
            .command(&["mcp", "--config", project.to_str().unwrap()])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        scratch.children.push(child.id());
        let stdout = child.stdout.take().unwrap();
        let (sender, lines) = unbounded();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { return };
                let message = serde_json::from_str::<Value>(&line).map_err(|_| line);
                if sender.send(message).is_err() {
                    return;
                }
            }
        });
        let mut session = Self {
            child,
            lines,
            next_id: 0,
        };
        session.request(
            "initialize",
            serde_json::json!({
                "protocolVersion": MCP_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "cli-exec", "version": "0" },
            }),
        );
        session.write(&serde_json::json!({
            "jsonrpc": JSON_RPC_VERSION,
            "method": "notifications/initialized",
        }));
        session
    }

    /// Writes one message to the server.
    fn write(&mut self, message: &Value) {
        let stdin = self.child.stdin.as_mut().unwrap();
        writeln!(stdin, "{message}").unwrap();
        stdin.flush().unwrap();
    }

    /// Sends `method` with `params` and returns the whole response.
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.write(&serde_json::json!({
            "jsonrpc": JSON_RPC_VERSION,
            "id": id,
            "method": method,
            "params": params,
        }));
        loop {
            let message = self
                .lines
                .recv_timeout(WAIT)
                .unwrap_or_else(|_| panic!("no answer to {method}"))
                .unwrap_or_else(|line| panic!("stdout carried a non-JSON line: {line:?}"));
            if message["id"] == id {
                return message;
            }
        }
    }

    /// Calls the tool `name` with `arguments` and returns the response.
    fn call(&mut self, name: &str, arguments: Value) -> Value {
        self.request(
            "tools/call",
            serde_json::json!({ "name": name, "arguments": arguments }),
        )
    }

    /// Calls the tool `name` with `arguments` and returns its result,
    /// failing on an error.
    fn result(&mut self, name: &str, arguments: Value) -> Value {
        let response = self.call(name, arguments);
        assert!(response["error"].is_null(), "{name} failed: {response}");
        response["result"].clone()
    }

    /// The text of the first content block of the tool call's result.
    fn text(&mut self, name: &str, arguments: Value) -> String {
        result_text(&self.result(name, arguments))
    }

    /// Calls the tool `name` with `arguments`, which the caller got wrong,
    /// and returns its result, failing unless it is flagged as an error.
    fn mistake(&mut self, name: &str, arguments: Value) -> Value {
        let result = self.result(name, arguments);
        assert_eq!(result["isError"], true, "{name} succeeded: {result}");
        result
    }
}

/// The text of the first content block of a tool result.
fn result_text(result: &Value) -> String {
    result["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// Pids of the zombie children of `parent`: exited, never reaped.
fn zombie_children(parent: u32) -> Vec<u32> {
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .filter(|pid| {
            fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
                let mut fields = stat
                    .rsplit_once(") ")
                    .map(|(_, rest)| rest.split_whitespace())
                    .into_iter()
                    .flatten();
                let state = fields.next();
                let ppid = fields.next().and_then(|ppid| ppid.parse::<u32>().ok());
                state == Some("Z") && ppid == Some(parent)
            })
        })
        .collect()
}

impl Drop for McpSession {
    /// Closes the server's input, which ends it, and reaps it.
    fn drop(&mut self) {
        drop(self.child.stdin.take());
        let _ = self.child.wait();
    }
}

/// With no service and no other muster running, an agent's MCP server starts
/// the service and drives the project's processes by name: it lists them,
/// starts, types into, reads, restarts and stops them, and an unknown name
/// is refused with the valid ones.
#[test]
fn mcp_tools_work_with_no_tui_and_the_service_autostarted() {
    let mut scratch = Scratch::new();
    let project = scratch.project("shop", MCP_PROJECT);
    assert!(processes_under(SERVICE_SUBCOMMAND, scratch.root.path()).is_empty());
    let mut mcp = McpSession::start(&mut scratch, &project);

    let tools = mcp.request("tools/list", serde_json::json!({}))["result"]["tools"].clone();
    for tool in PROCESS_TOOLS {
        let schema = &tools
            .as_array()
            .unwrap()
            .iter()
            .find(|listed| listed["name"] == tool)
            .unwrap_or_else(|| panic!("no tool {tool}: {tools}"))["inputSchema"];
        assert!(schema["properties"]["name"].is_object(), "{tool}: {schema}");
        assert!(schema["properties"]["id"].is_null(), "{tool}: {schema}");
    }

    let listed = mcp.result("process_list", serde_json::json!({}))["structuredContent"].clone();
    assert_eq!(listed["project"], project.to_str().unwrap());
    let names: Vec<&str> = listed["processes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|process| {
            assert!(process["id"].is_null(), "no pane id on the wire: {process}");
            process["name"].as_str().unwrap()
        })
        .collect();
    assert_eq!(names, ["echo box", "café"]);
    assert!(
        !processes_under(SERVICE_SUBCOMMAND, scratch.root.path()).is_empty(),
        "the first live tool started the service"
    );

    let started = mcp.result("process_start", serde_json::json!({ "name": "echo box" }));
    assert_eq!(started["isError"], false, "{started}");
    let report = &started["structuredContent"];
    assert_eq!(report["action"], "start");
    assert_eq!(report["processes"][0]["name"], "echo box");
    assert_eq!(report["processes"][0]["outcome"], "started");
    assert_eq!(report["processes"][0]["status"]["state"], "running");

    assert_eq!(
        mcp.text(
            "process_send",
            serde_json::json!({ "name": "echo box", "text": "hello muster", "enter": true }),
        ),
        "sent"
    );
    wait_until("the echo to reach the screen", || {
        mcp.text("process_output", serde_json::json!({ "name": "echo box" }))
            .contains("hello muster")
    });

    let accented = mcp.result(
        "process_start",
        serde_json::json!({ "name": "cafe\u{301}" }),
    );
    assert_eq!(
        accented["structuredContent"]["processes"][0]["name"],
        "caf\u{e9}"
    );

    let unknown = mcp.mistake("process_stop", serde_json::json!({ "name": "echo" }));
    let text = result_text(&unknown);
    assert!(
        text.contains("no process 'echo'") && text.contains("'echo box'"),
        "{unknown}"
    );
    let error = &unknown["structuredContent"]["error"];
    assert_eq!(error["kind"], "unknown-process");
    assert_eq!(error["candidates"], serde_json::json!(["echo box", "café"]));

    let restarted = mcp.result("process_restart", serde_json::json!({ "name": "echo box" }));
    assert_eq!(
        restarted["structuredContent"]["processes"][0]["outcome"],
        "restarted"
    );
    assert_eq!(
        mcp.result("session_list", serde_json::json!({}))["structuredContent"],
        serde_json::json!({ "sessions": [] })
    );
    for name in ["echo box", "café"] {
        let stopped = mcp.result("process_stop", serde_json::json!({ "name": name }));
        assert_eq!(
            stopped["structuredContent"]["processes"][0]["outcome"], "stopped",
            "{stopped}"
        );
    }
    assert_eq!(
        mcp.result("process_list", serde_json::json!({}))["structuredContent"]["processes"][1]["last_exit"]
            ["signal"],
        "sigterm",
        "a signal is named by its POSIX name, whatever the locale"
    );

    let stopped = mcp.mistake(
        "process_send",
        serde_json::json!({ "name": "echo box", "text": "anyone?" }),
    );
    let text = result_text(&stopped);
    assert!(text.contains("call process_start"), "{stopped}");
    assert!(!text.contains("muster start"), "{stopped}");
    assert_eq!(stopped["structuredContent"]["error"]["kind"], "not-running");
    let nobody = mcp.mistake(
        "process_send",
        serde_json::json!({ "name": "nobody", "text": "anyone?" }),
    );
    assert!(result_text(&nobody).contains("'echo box'"), "{nobody}");
    assert_eq!(
        nobody["structuredContent"]["error"]["candidates"],
        serde_json::json!(["echo box", "café"])
    );

    let stop = scratch.muster(&["service", "stop"]);
    assert!(stop.status.success(), "{}", shown(&stop));
    let server = mcp.child.id();
    wait_until("the stopped service to be reaped", || {
        zombie_children(server).is_empty()
    });
}

/// Rows of the pseudo-terminal a prompting command runs on.
const PTY_ROWS: u16 = 40;
/// Columns of that pseudo-terminal.
const PTY_COLS: u16 = 120;
/// Bytes read from a pseudo-terminal's master at a time.
const PTY_CHUNK: usize = 4096;
/// A terminal type that draws color, so the interactive layout is chosen.
const COLOR_TERM: &str = "xterm-256color";
/// Keys a person types at a prompt.
const ENTER: &[u8] = b"\r";
const ARROW_DOWN: &[u8] = b"\x1b[B";
const CTRL_C: &[u8] = b"\x03";
/// The shipped detach key, `ctrl-\`.
const DETACH: &[u8] = b"\x1c";

/// A pseudo-terminal a command runs on as a person's terminal: the test
/// types on its master side and collects everything drawn there, so no test
/// touches a real terminal.
struct Pty {
    master: fs::File,
    slave: OwnedFd,
    drawn: Arc<Mutex<Vec<u8>>>,
}

impl Pty {
    /// Opens a pseudo-terminal and starts collecting what is drawn on it.
    fn open() -> Self {
        Self::open_sized(PTY_ROWS, PTY_COLS)
    }

    /// Opens a `rows` by `cols` pseudo-terminal and starts collecting what is
    /// drawn on it.
    fn open_sized(rows: u16, cols: u16) -> Self {
        let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).unwrap();
        grantpt(&master).unwrap();
        unlockpt(&master).unwrap();
        let path = ptsname(&master, Vec::new()).unwrap();
        let slave: OwnedFd = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(libc::O_NOCTTY)
            .open(path.to_str().unwrap())
            .unwrap()
            .into();
        tcsetwinsize(&master, Winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        })
        .unwrap();
        let pty = Self {
            master: fs::File::from(master),
            slave,
            drawn: Arc::new(Mutex::new(Vec::new())),
        };
        let mut reader = pty.master.try_clone().unwrap();
        let drawn = Arc::clone(&pty.drawn);
        thread::spawn(move || {
            let mut chunk = [0; PTY_CHUNK];
            while let Ok(read) = reader.read(&mut chunk)
                && read > 0
            {
                drawn
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .extend_from_slice(&chunk[..read]);
            }
        });
        pty
    }

    /// Runs `muster <args>` over `scratch` from `folder`, with this terminal
    /// as its input, output and error.
    fn spawn(&self, scratch: &mut Scratch, folder: &Path, args: &[&str]) -> Child {
        let stdio = || Stdio::from(self.slave.try_clone().unwrap());
        let child = scratch
            .command(args)
            .current_dir(folder)
            .env("TERM", COLOR_TERM)
            .env_remove("NO_COLOR")
            .stdin(stdio())
            .stdout(stdio())
            .stderr(stdio())
            .spawn()
            .unwrap();
        scratch.children.push(child.id());
        child
    }

    /// Types `keys` as a person would, once the command reads keys raw, so a
    /// Ctrl-C reaches it as a key rather than as a signal.
    fn type_keys(&self, keys: &[u8]) {
        wait_until("the terminal to read keys raw", || {
            !tcgetattr(&self.slave)
                .unwrap()
                .local_modes
                .contains(LocalModes::ICANON)
        });
        (&self.master).write_all(keys).unwrap();
    }

    /// Runs `muster <args>` like [`Pty::spawn`], but as the leader of its
    /// own session with this terminal as its controlling one, so a Ctrl-C
    /// typed while it is not reading keys reaches it as `SIGINT`.
    fn spawn_controlling(&self, scratch: &mut Scratch, folder: &Path, args: &[&str]) -> Child {
        let stdio = || Stdio::from(self.slave.try_clone().unwrap());
        let mut command = scratch.command(args);
        command
            .current_dir(folder)
            .env("TERM", COLOR_TERM)
            .env_remove("NO_COLOR")
            .stdin(stdio())
            .stdout(stdio())
            .stderr(stdio());
        // SAFETY: `setsid` and `ioctl` are async-signal-safe and touch only
        // the child's own session and its standard input.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().unwrap();
        scratch.children.push(child.id());
        child
    }

    /// Types Ctrl-C at once, whether or not the command reads keys raw.
    fn interrupt(&self) {
        (&self.master).write_all(CTRL_C).unwrap();
    }

    /// Everything drawn so far, escape sequences included.
    fn raw(&self) -> String {
        let bytes = self
            .drawn
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// Everything drawn so far, as text with the styling sequences removed.
    fn drawn(&self) -> String {
        unstyled(&self.raw())
    }

    /// Waits until `needle` has been drawn `times` times in all.
    fn wait_for(&self, needle: &str, times: usize) {
        wait_until(&format!("{needle:?} drawn {times} times"), || {
            self.drawn().matches(needle).count() >= times
        });
    }

    /// Waits for `child` to exit and returns its exit code.
    fn exit_code(child: &mut Child) -> Option<i32> {
        let started = Instant::now();
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                return status.code();
            }
            assert!(started.elapsed() < WAIT, "the command did not exit");
            thread::sleep(POLL);
        }
    }
}

/// `text` with its CSI and OSC escape sequences removed.
fn unstyled(text: &str) -> String {
    let mut plain = String::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            plain.push(c);
            continue;
        }
        match chars.next() {
            Some('[') => {
                for c in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&c) {
                        break;
                    }
                }
            },
            Some(']') => {
                for c in chars.by_ref() {
                    if c == '\x07' {
                        break;
                    }
                }
            },
            _ => {},
        }
    }
    plain
}

/// A project with an agent and a command, both autostarting.
const AGENT_PROJECT: &str = "agents:\n  - name: bot\n    command: sleep 600\n    autostart: true\nterminals: []\ncommands:\n  - name: api\n    command: sleep 600\n    autostart: true\n";

/// Without a terminal to ask on, a command that needs a name or an answer
/// fails as invalid input naming what to pass, also in JSON, and changes
/// nothing; a missing name is refused before the service is reached, and
/// `down --agents` asks nothing when no agent runs.
#[test]
fn a_question_nobody_can_answer_names_the_argument_to_pass() {
    let scratch = Scratch::new();
    let project = scratch.project("shop", AGENT_PROJECT);
    let config = project.to_str().unwrap();
    let unnamed = scratch.muster(&["-c", config, "stop"]);
    assert_eq!(
        unnamed.status.code(),
        Some(INVALID_INPUT),
        "{}",
        shown(&unnamed)
    );
    assert!(
        processes_under(SERVICE_SUBCOMMAND, scratch.root.path()).is_empty(),
        "a missing name is refused before the service starts"
    );
    let up = scratch.muster(&["-c", config, "up"]);
    assert!(up.status.success(), "{}", shown(&up));

    for command in ["start", "stop", "restart", "logs"] {
        let output = scratch.muster(&["-c", config, command]);
        assert_eq!(
            output.status.code(),
            Some(INVALID_INPUT),
            "{command}: {}",
            shown(&output)
        );
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("pass its name"),
            "{command}: {}",
            shown(&output)
        );
    }
    let (output, failure) = scratch.json(&["-c", config, "stop"]);
    assert_eq!(output.status.code(), Some(INVALID_INPUT));
    assert_eq!(failure["error"]["kind"], "invalid-input", "{failure}");

    let down = scratch.muster(&["-c", config, "down", "--agents"]);
    assert_eq!(down.status.code(), Some(INVALID_INPUT), "{}", shown(&down));
    assert!(
        String::from_utf8_lossy(&down.stderr).contains("pass --yes"),
        "{}",
        shown(&down)
    );
    assert_eq!(scratch.row(&project, "bot")["state"], "running");
    assert_eq!(scratch.row(&project, "api")["state"], "running");

    let (down, report) = scratch.json(&["-c", config, "down", "--agents", "--yes"]);
    assert!(down.status.success(), "{}", shown(&down));
    assert!(
        report["processes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|process| process["outcome"] == "stopped"),
        "{report}"
    );

    let commands_only = scratch.project("plain", SINGLE_PROJECT);
    let commands_only = commands_only.to_str().unwrap();
    let up = scratch.muster(&["-c", commands_only, "up"]);
    assert!(up.status.success(), "{}", shown(&up));
    let down = scratch.muster(&["-c", commands_only, "down", "--agents"]);
    assert_eq!(
        down.status.code(),
        Some(INVALID_INPUT),
        "without a terminal --yes is always needed: {}",
        shown(&down)
    );
}

/// Without a terminal, `down --agents` without `--yes` is refused before
/// the service is reached.
#[test]
fn down_agents_without_yes_is_refused_before_the_service_starts() {
    let scratch = Scratch::new();
    let project = scratch.project("shop", AGENT_PROJECT);
    let config = project.to_str().unwrap();

    let down = scratch.muster(&["-c", config, "down", "--agents"]);

    assert_eq!(down.status.code(), Some(INVALID_INPUT), "{}", shown(&down));
    assert!(
        String::from_utf8_lossy(&down.stderr).contains("pass --yes"),
        "{}",
        shown(&down)
    );
    assert!(
        processes_under(SERVICE_SUBCOMMAND, scratch.root.path()).is_empty(),
        "no service was started"
    );
}

/// Ctrl-C under the spinner still closes the frame after a question was
/// asked in it: after the process chooser, and after the `down --agents`
/// confirmation.
#[test]
fn ctrl_c_under_the_spinner_after_a_question_closes_the_frame() {
    let mut scratch = Scratch::new();
    let slow_agents = format!(
        "agents:\n  - name: bot\n    command: sleep 600\n    autostart: true\n{}",
        SLOW_STOP_PROJECT.trim_start_matches("agents: []\n")
    );
    let project = scratch.project("shop", &slow_agents);
    let config = project.to_str().unwrap();
    let folder = project.parent().unwrap().to_path_buf();
    let interrupted = |pty: &Pty, child: &mut Child| {
        pty.wait_for("stopping", 1);
        pty.interrupt();
        assert_eq!(Pty::exit_code(child), Some(INTERRUPTED), "{}", pty.drawn());
        assert!(pty.drawn().contains("└  interrupted"), "{}", pty.drawn());
        let raw = pty.raw();
        assert!(
            raw.rfind("\x1b[?25h") >= raw.rfind("\x1b[?25l"),
            "the cursor is shown again: {raw:?}"
        );
    };

    let up = scratch.muster(&["-c", config, "up"]);
    assert!(up.status.success(), "{}", shown(&up));
    let pty = Pty::open();
    let mut stop = pty.spawn_controlling(&mut scratch, &folder, &["stop"]);
    pty.wait_for("Which process?", 1);
    pty.type_keys(ARROW_DOWN);
    pty.type_keys(ENTER);
    interrupted(&pty, &mut stop);
    wait_until("slow to stop", || {
        scratch.row(&project, "slow")["state"] != "running"
    });

    let up = scratch.muster(&["-c", config, "up"]);
    assert!(up.status.success(), "{}", shown(&up));
    let pty = Pty::open();
    let mut down = pty.spawn_controlling(&mut scratch, &folder, &["down", "--agents"]);
    pty.wait_for("Stop this project's agents too?", 1);
    pty.type_keys(b"y");
    interrupted(&pty, &mut down);
}

/// Ctrl-C before the command sent its request cancels it: nothing is sent,
/// and the frame says nothing was changed.
#[test]
fn ctrl_c_before_the_request_is_sent_cancels_it() {
    let mut scratch = Scratch::new();
    let project = scratch.project("shop", SINGLE_PROJECT);
    let folder = project.parent().unwrap().to_path_buf();
    let asked = slow_status_service(&scratch, project);

    let pty = Pty::open();
    let mut up = pty.spawn_controlling(&mut scratch, &folder, &["up"]);
    wait_until("the status to be asked for", || {
        asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&"ps")
    });
    pty.interrupt();

    assert_eq!(
        Pty::exit_code(&mut up),
        Some(INTERRUPTED),
        "{}",
        pty.drawn()
    );
    assert!(
        pty.drawn().contains("└  cancelled; nothing was changed"),
        "{}",
        pty.drawn()
    );
    assert!(
        !asked
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .contains(&"up"),
        "the request was never sent"
    );
}

/// A failure after `logs` closed its question's frame is printed on its own,
/// without closing the frame a second time.
#[test]
fn a_logs_failure_after_the_question_closes_no_frame_twice() {
    let mut scratch = Scratch::new();
    let project = scratch.project("shop", SINGLE_PROJECT);
    let folder = project.parent().unwrap().to_path_buf();
    scripted_service(&scratch, project);

    let pty = Pty::open();
    let mut logs = pty.spawn(&mut scratch, &folder, &["logs"]);
    pty.wait_for("Which process?", 1);
    pty.type_keys(ENTER);

    assert_eq!(
        Pty::exit_code(&mut logs),
        Some(SERVICE_UNAVAILABLE),
        "{}",
        pty.drawn()
    );
    let drawn = pty.drawn();
    assert_eq!(drawn.matches("└  api").count(), 1, "{drawn}");
    assert!(!drawn.contains("└  the muster service"), "{drawn}");
    assert!(drawn.contains("✗ muster: the muster service"), "{drawn}");
}

/// At a terminal, `up` is framed with the project's name and closed with a
/// summary, and `down --agents` asks first while an agent runs: Ctrl-C
/// cancels it as a usage error, a no stops nothing and succeeds, and a yes
/// stops the project.
#[test]
fn up_is_framed_and_down_agents_asks_first() {
    let mut scratch = Scratch::new();
    let project = scratch.project("shop", AGENT_PROJECT);
    let folder = project.parent().unwrap().to_path_buf();

    let pty = Pty::open();
    let mut up = pty.spawn(&mut scratch, &folder, &["up"]);
    assert_eq!(Pty::exit_code(&mut up), Some(0), "{}", pty.drawn());
    let drawn = pty.drawn();
    assert!(drawn.contains("muster up · shop"), "{drawn}");
    assert!(drawn.contains("running (pid"), "{drawn}");
    assert!(drawn.contains("2 ok"), "{drawn}");

    let pty = Pty::open();
    let mut down = pty.spawn(&mut scratch, &folder, &["down", "--agents"]);
    pty.wait_for("Stop this project's agents too?", 1);
    pty.type_keys(CTRL_C);
    assert_eq!(Pty::exit_code(&mut down), Some(USAGE), "{}", pty.drawn());
    let drawn = pty.drawn();
    assert_eq!(
        drawn.matches("cancelled").count(),
        1,
        "closed once: {drawn}"
    );
    assert!(drawn.contains("nothing was changed"), "{drawn}");
    assert_eq!(scratch.row(&project, "bot")["state"], "running");
    assert_eq!(scratch.row(&project, "api")["state"], "running");

    let pty = Pty::open();
    let mut down = pty.spawn(&mut scratch, &folder, &["down", "--agents"]);
    pty.wait_for("Stop this project's agents too?", 1);
    pty.type_keys(b"n");
    assert_eq!(Pty::exit_code(&mut down), Some(0), "{}", pty.drawn());
    assert!(pty.drawn().contains("nothing stopped"), "{}", pty.drawn());
    assert_eq!(scratch.row(&project, "bot")["state"], "running");
    assert_eq!(scratch.row(&project, "api")["state"], "running");

    let pty = Pty::open();
    let mut down = pty.spawn(&mut scratch, &folder, &["down", "--agents"]);
    pty.wait_for("Stop this project's agents too?", 1);
    pty.type_keys(b"y");
    assert_eq!(Pty::exit_code(&mut down), Some(0), "{}", pty.drawn());
    assert!(pty.drawn().contains("stopped"), "{}", pty.drawn());
    assert_ne!(scratch.row(&project, "bot")["state"], "running");
    assert_ne!(scratch.row(&project, "api")["state"], "running");
}

/// Ctrl-C while `down` waits under its spinner ends the command at once
/// with the frame closed and exit code 130, and the service finishes the
/// stop on its own.
#[test]
fn ctrl_c_under_the_spinner_closes_the_frame() {
    let mut scratch = Scratch::new();
    let project = scratch.project("shop", SLOW_STOP_PROJECT);
    let config = project.to_str().unwrap();
    let folder = project.parent().unwrap().to_path_buf();
    let up = scratch.muster(&["-c", config, "up"]);
    assert!(up.status.success(), "{}", shown(&up));

    let pty = Pty::open();
    let mut down = pty.spawn_controlling(&mut scratch, &folder, &["down"]);
    pty.wait_for("stopping slow", 1);
    pty.interrupt();
    assert_eq!(
        Pty::exit_code(&mut down),
        Some(INTERRUPTED),
        "{}",
        pty.drawn()
    );
    let drawn = pty.drawn();
    assert!(drawn.contains("└  interrupted"), "{drawn}");
    assert!(
        drawn.ends_with("\r\n"),
        "the prompt starts on a line of its own: {drawn:?}"
    );
    let raw = pty.raw();
    assert!(
        raw.rfind("\x1b[?25h") >= raw.rfind("\x1b[?25l"),
        "the cursor is shown again: {raw:?}"
    );
    wait_until("the service to finish the stop", || {
        scratch.row(&project, "slow")["state"] != "running"
    });
}

/// At a terminal, `attach` with no name offers only the running processes,
/// attaches to the one chosen and closes its frame once detached; with none
/// running it says so and fails.
#[test]
fn attach_without_a_name_offers_the_running_processes() {
    let mut scratch = Scratch::new();
    let project = scratch.project("shop", PROJECT);
    let config = project.to_str().unwrap();
    let folder = project.parent().unwrap().to_path_buf();

    let pty = Pty::open();
    let mut attach = pty.spawn(&mut scratch, &folder, &["attach"]);
    assert_eq!(
        Pty::exit_code(&mut attach),
        Some(ACTION_FAILED),
        "{}",
        pty.drawn()
    );
    assert!(pty.drawn().contains("no process"), "{}", pty.drawn());

    let up = scratch.muster(&["-c", config, "up"]);
    assert!(up.status.success(), "{}", shown(&up));
    let pty = Pty::open();
    let mut attach = pty.spawn(&mut scratch, &folder, &["attach"]);
    pty.wait_for("Which process?", 1);
    let drawn = pty.drawn();
    assert!(drawn.contains("command · running"), "{drawn}");
    assert!(
        !drawn.contains("○ api") && !drawn.contains("● api"),
        "{drawn}"
    );
    pty.type_keys(ARROW_DOWN);
    pty.type_keys(ENTER);
    pty.wait_for("attached to café", 1);
    pty.type_keys(DETACH);
    assert_eq!(Pty::exit_code(&mut attach), Some(0), "{}", pty.drawn());
    assert!(
        pty.drawn().contains("detached from café"),
        "{}",
        pty.drawn()
    );
}

/// Rows of the terminal the surviving attach runs on.
const ATTACH_ROWS: u16 = 50;
/// Columns of that terminal.
const ATTACH_COLS: u16 = 200;

/// The PTY size of `name`'s holder in `project`, as a read-only client is
/// told it.
fn holder_size(scratch: &Scratch, project: &Path, name: &str) -> PtySize {
    let ServiceResponse::Located(located) = scratch.send(&ServiceRequest::Locate {
        project: project.to_path_buf(),
        name: ProcessName::try_new(name).unwrap(),
    }) else {
        panic!("a running process is located");
    };
    let mut stream = UnixStream::connect(located.socket()).unwrap();
    greet(
        &mut stream,
        located.socket(),
        ClientRole::Attach { read_only: true },
        WAIT,
    )
    .unwrap()
    .size()
}

/// An attached terminal outlives a service restart untouched: the successor
/// neither resizes the process back to the default size nor loses the count
/// of attached terminals, which the holder keeps. A SIGKILLed attach, which
/// never says it left, stops being counted all the same.
#[test]
fn an_attach_survives_a_service_restart_and_a_killed_attach_is_uncounted() {
    let mut scratch = Scratch::new();
    let project = scratch.project("shop", SINGLE_PROJECT);
    let config = project.to_str().unwrap();
    let folder = project.parent().unwrap().to_path_buf();
    let up = scratch.muster(&["-c", config, "up"]);
    assert!(up.status.success(), "{}", shown(&up));
    let attached_size = PtySize::builder()
        .rows(Rows::new(ATTACH_ROWS))
        .cols(Cols::new(ATTACH_COLS))
        .build();

    let pty = Pty::open_sized(ATTACH_ROWS, ATTACH_COLS);
    let mut attach = pty.spawn(&mut scratch, &folder, &["attach", "api"]);
    pty.wait_for("attached to api", 1);
    wait_until("the attach to be counted", || {
        scratch.row(&project, "api")["attached"] == 1
    });
    wait_until("the attach to size the process", || {
        holder_size(&scratch, &project, "api") == attached_size
    });

    let stop = scratch.muster(&["service", "stop"]);
    assert!(stop.status.success(), "{}", shown(&stop));
    wait_until("the successor to adopt the process", || {
        scratch.row(&project, "api")["state"] == "running"
    });
    assert_eq!(scratch.row(&project, "api")["attached"], 1);
    assert_eq!(holder_size(&scratch, &project, "api"), attached_size);

    attach.kill().unwrap();
    attach.wait().unwrap();
    wait_until("the killed attach to be uncounted", || {
        scratch.row(&project, "api")["attached"] == 0
    });
}

/// At a terminal, `logs` with no name frames its question and closes the
/// frame before the log is printed.
#[test]
fn logs_without_a_name_closes_its_frame_before_the_log() {
    let mut scratch = Scratch::new();
    let project = scratch.project("shop", PROJECT);
    let config = project.to_str().unwrap();
    let folder = project.parent().unwrap().to_path_buf();
    let up = scratch.muster(&["-c", config, "up"]);
    assert!(up.status.success(), "{}", shown(&up));
    wait_until("the dev server to write", || {
        String::from_utf8_lossy(&scratch.muster(&["-c", config, "logs", "dev server"]).stdout)
            .contains("dev-ready")
    });

    let pty = Pty::open();
    let mut logs = pty.spawn(&mut scratch, &folder, &["logs"]);
    pty.wait_for("Which process?", 1);
    pty.type_keys(ENTER);
    assert_eq!(Pty::exit_code(&mut logs), Some(0), "{}", pty.drawn());
    let drawn = pty.drawn();
    let closed = drawn.rfind("└  dev server").expect(&drawn);
    let log = drawn.rfind("dev-ready").expect(&drawn);
    assert!(closed < log, "{drawn}");
}

/// Data a command prints is never framed, even at a terminal: a note's
/// body, a listing and the completions line come out raw.
#[test]
fn data_output_is_never_framed() {
    let mut scratch = Scratch::new();
    let project = scratch.project("shop", SINGLE_PROJECT);
    let folder = project.parent().unwrap().to_path_buf();
    let set = scratch
        .command(&["note", "set", "plan", "ship it"])
        .current_dir(&folder)
        .output()
        .unwrap();
    assert!(set.status.success(), "{}", shown(&set));

    for args in [
        &["note", "get", "plan"][..],
        &["note"][..],
        &["completions", "bash"][..],
    ] {
        let pty = Pty::open();
        let mut child = pty.spawn(&mut scratch, &folder, args);
        assert_eq!(
            Pty::exit_code(&mut child),
            Some(0),
            "{args:?}: {}",
            pty.drawn()
        );
        let drawn = pty.drawn();
        assert!(
            !drawn.contains('┌') && !drawn.contains('└'),
            "{args:?}: {drawn}"
        );
    }
}

/// At a terminal, `init` asks for each process and writes them, explicit,
/// into the new muster.yml.
#[test]
fn init_asks_for_each_process() {
    let mut scratch = Scratch::new();
    let folder = scratch.root.path().join("fresh");
    fs::create_dir_all(&folder).unwrap();

    let pty = Pty::open();
    let mut init = pty.spawn(&mut scratch, &folder, &["init"]);
    let answers: [(&str, &[&[u8]]); 12] = [
        ("Process name", &[b"web\r"]),
        ("Kind", &[ARROW_DOWN, ENTER]),
        ("Command", &[b"npm run dev\r"]),
        ("Start it with `muster up`?", &[b"n"]),
        ("Restart it", &[ARROW_DOWN, ENTER]),
        ("Add another process?", &[b"y"]),
        ("Process name", &[b"shell\r"]),
        ("Kind", &[ENTER]),
        ("Command", &[ENTER]),
        ("Start it with `muster up`?", &[b"y"]),
        ("Restart it", &[ENTER]),
        ("Add another process?", &[b"n"]),
    ];
    for (asked, (prompt, keys)) in answers.iter().enumerate() {
        let times = answers[..=asked]
            .iter()
            .filter(|(other, _)| other == prompt)
            .count();
        pty.wait_for(&format!("◆  {prompt}"), times);
        for key in *keys {
            pty.type_keys(key);
        }
        pty.wait_for(&format!("◇  {prompt}"), times);
    }
    assert_eq!(Pty::exit_code(&mut init), Some(0), "{}", pty.drawn());
    assert!(
        pty.drawn().contains("created muster.yml"),
        "{}",
        pty.drawn()
    );

    let written: Value =
        serde_yaml_ng::from_str(&fs::read_to_string(folder.join("muster.yml")).unwrap()).unwrap();
    assert_eq!(written["agents"], serde_json::json!([]));
    assert_eq!(
        written["commands"],
        serde_json::json!([{
            "name": "web",
            "command": "npm run dev",
            "working_dir": null,
            "description": null,
            "restart": "on-failure",
            "stop": null,
            "autostart": false,
        }])
    );
    assert_eq!(written["terminals"][0]["name"], "shell");
    assert_eq!(written["terminals"][0]["command"], Value::Null);
    assert_eq!(written["terminals"][0]["autostart"], true);
    assert_eq!(written["terminals"][0]["restart"], "never");
}
