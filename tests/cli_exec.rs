//! The CLI end to end: the built `muster` binary run as a user would, over
//! isolated XDG roots with desktop notifications off, its projects added to
//! the database through the CLI itself, starting its own service on demand,
//! replacing an older service, refusing a newer one, and reporting what
//! happened to processes.

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

use crossbeam_channel::{Receiver, unbounded};
use muster::{
    adapter::{
        holder::{
            ClientRole, ExecHolderLauncher, HOLDER_SUBCOMMAND, HolderConfig, HolderDirs,
            HolderLauncher, HolderRunner, client::greet,
        },
        service::{
            BatchOutcome, LaunchEnvironment, Notice, NoticeLevel, ProcessAction, ProcessStatus,
            ProjectPorts, ProjectStatus, Refusal, ServiceDeps, ServiceError, ServiceRequest,
            ServiceResponse, ServiceRuntime, Singleton,
            protocol::{self, SERVICE_SUBCOMMAND},
        },
        store::SqliteStore,
    },
    domain::{
        port::{Notifier, ProcessMetrics},
        process::ProcessState,
        project::Project,
        pty::{PtyError, PtySize},
        value::{Cols, ProcessName, ProjectName, ProjectRoot, Rows},
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
/// Bound on every wait; reaching it fails the test.
const WAIT: Duration = Duration::from_secs(30);
/// Interval between checks of a polled condition.
const POLL: Duration = Duration::from_millis(20);
/// Short root for test sockets, inside `sun_path`.
const SHORT_TEMP_ROOT: &str = "/tmp";
/// Prefix of the scratch directory.
const SCRATCH_PREFIX: &str = "mc-";
/// Directory muster keeps its state under, inside the XDG state root.
const APP_DIR: &str = "muster";
/// muster's database inside its state directory.
const DATABASE_FILE: &str = "muster.db";
/// The stack file `export` and `import` use by default.
const STACK_FILE: &str = "muster.yml";
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
/// The default stop grace every test process has unless it says otherwise.
const GRACE: &str = "5s";
/// Why the scripted service leaves `api` alone on `up`.
const REFUSAL_REASON: &str = "cannot start: an earlier run may still be running as pid 4242";
/// The variable a started process reads from the command that started it.
const CALLER_VARIABLE: &str = "CLI_EXEC_CALLER_MARK";
/// The schema version a newer muster would have written.
const FUTURE_SCHEMA: u32 = 999;

/// One process of a stack file.
struct Entry<'a> {
    /// Its name.
    name: &'a str,
    /// The shell command it runs.
    command: &'a str,
    /// Whether `muster up` starts it.
    autostart: bool,
    /// Its restart policy, as the file spells it.
    restart: &'a str,
    /// Its stop grace, as the file spells it.
    grace: &'a str,
}

/// A process that autostarts unless `autostart` says otherwise, never
/// restarts, and stops with the default grace.
const fn entry<'a>(name: &'a str, command: &'a str, autostart: bool) -> Entry<'a> {
    Entry {
        name,
        command,
        autostart,
        restart: "never",
        grace: GRACE,
    }
}

/// The stack file holding `entries`, every value quoted so commands with
/// shell punctuation read back as they are.
fn stack(entries: &[Entry<'_>]) -> String {
    let quoted = |text: &str| serde_json::to_string(text).unwrap();
    let mut file = String::from("version: 1\nprocesses:\n");
    for entry in entries {
        file.push_str(&format!(
            "  - name: {}\n    command: {}\n    autostart: {}\n    restart: {}\n    stop:\n      signal: terminate\n      grace_period: {}\n",
            quoted(entry.name),
            quoted(entry.command),
            entry.autostart,
            entry.restart,
            entry.grace,
        ));
    }
    file
}

/// Two long runners with a space and an accent in their names, and a third
/// that does not autostart.
fn shop() -> String {
    stack(&[
        entry("dev server", "echo dev-ready; sleep 600", true),
        entry("café", "sleep 600", true),
        entry("api", "echo api-ran; sleep 600", false),
    ])
}

/// A project whose second autostart process fails at once with status 3.
fn failing() -> String {
    stack(&[
        entry("web", "sleep 600", true),
        entry("broken", "echo oops; exit 3", true),
    ])
}

/// One long runner.
fn single() -> String {
    stack(&[entry("api", "sleep 600", true)])
}

/// A runner that takes a second to stop and a neighbour that crash-loops.
fn crash_loop() -> String {
    stack(&[
        entry(
            "api",
            "trap 'sleep 1; exit 0' TERM; while :; do sleep 0.1; done",
            true,
        ),
        Entry {
            restart: "always",
            ..entry("flaky", "sleep 0.1; exit 1", true)
        },
    ])
}

/// A process that ignores the stop signal for its one-second grace.
fn slow_stop() -> String {
    stack(&[Entry {
        grace: "1s",
        ..entry("slow", "trap '' TERM; while :; do sleep 0.1; done", true)
    }])
}

/// A process that writes a line every tenth of a second.
fn ticking() -> String {
    stack(&[entry(
        "ticker",
        "while :; do echo tick; sleep 0.1; done",
        true,
    )])
}

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
    /// A fresh scratch directory short enough to bind sockets in, with
    /// desktop notifications turned off in its database.
    fn new() -> Self {
        let root = tempfile::Builder::new()
            .prefix(SCRATCH_PREFIX)
            .tempdir_in(SHORT_TEMP_ROOT)
            .unwrap();
        let scratch = Self {
            root,
            children: Vec::new(),
        };
        let quiet = scratch.muster(&["config", "set", "desktop-notifications", "false"]);
        assert!(quiet.status.success(), "{}", shown(&quiet));
        scratch
    }

    /// The scratch directory, which is no project.
    fn path(&self) -> PathBuf {
        self.root.path().canonicalize().unwrap()
    }

    /// The state directory every service over this scratch owns.
    fn state_dir(&self) -> PathBuf {
        self.root.path().join("state").join(APP_DIR)
    }

    /// The database every command over this scratch reads and writes.
    fn database(&self) -> PathBuf {
        self.state_dir().join(DATABASE_FILE)
    }

    /// The project in `folder`, registered with the processes of the stack
    /// file `stack` through `muster import`; returns its folder.
    fn project(&self, folder: &str, stack: &str) -> PathBuf {
        let dir = self.path().join(folder);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join(STACK_FILE), stack).unwrap();
        let import = self.muster(&["-p", dir.to_str().unwrap(), "import"]);
        assert!(import.status.success(), "{}", shown(&import));
        dir
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

    /// Runs `muster <args>` on the project in `project`.
    fn on(&self, project: &Path, args: &[&str]) -> Output {
        let mut all = vec!["-p", project.to_str().unwrap()];
        all.extend_from_slice(args);
        self.muster(&all)
    }

    /// Runs `muster <args>`, with `--json` appended, and parses what it
    /// printed.
    fn json(&self, args: &[&str]) -> (Output, Value) {
        let mut all = args.to_vec();
        all.push("--json");
        let output = self.muster(&all);
        (output.clone(), parsed(&output))
    }

    /// Runs `muster <args> --json` on the project in `project`.
    fn json_on(&self, project: &Path, args: &[&str]) -> (Output, Value) {
        let mut all = vec!["-p", project.to_str().unwrap()];
        all.extend_from_slice(args);
        self.json(&all)
    }

    /// The `ps` row of process `name` in `project`.
    fn row(&self, project: &Path, name: &str) -> Value {
        let (_, statuses) = self.json_on(project, &["ps"]);
        statuses[0]["processes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|process| process["name"] == name)
            .cloned()
            .unwrap_or_else(|| panic!("no row {name}: {statuses}"))
    }

    /// The stack file of `project`, as `export -` prints it.
    fn exported(&self, project: &Path) -> String {
        let export = self.on(project, &["export", "-"]);
        assert!(export.status.success(), "{}", shown(&export));
        String::from_utf8(export.stdout).unwrap()
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
            project: Some(root(project)),
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

/// The project root of the folder `project`.
fn root(project: &Path) -> ProjectRoot {
    ProjectRoot::try_new(project.to_path_buf()).unwrap()
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

/// What `output` printed on stdout.
fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// What `output` printed on stderr.
fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The ports of a service run in the test process: nothing of the user's
/// desktop or process table.
struct QuietPorts;

impl ProjectPorts for QuietPorts {
    fn notifier(&self) -> Option<Box<dyn Notifier>> {
        None
    }

    fn process_metrics(&self) -> Option<Box<dyn ProcessMetrics>> {
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

/// Runs a service reporting `version` in this process over `scratch`'s state
/// directory and database, launching real holders from the built binary,
/// held by [`GATE_HOLD`] when `held` is set.
fn in_process_service(
    scratch: &Scratch,
    version: &str,
    held: bool,
) -> JoinHandle<Result<(), ServiceError>> {
    let state_dir = scratch.state_dir();
    let database = scratch.database();
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
        let store = SqliteStore::open(&database).expect("the scratch database opens");
        let deps = ServiceDeps::builder()
            .version(version)
            .binary(PathBuf::from(IN_PROCESS_BINARY))
            .holders(Arc::new(HolderRunner::new(
                launcher,
                HolderDirs::under(&state_dir),
            )))
            .ports(Box::new(QuietPorts))
            .stacks(Box::new(store.clone()))
            .settings(Box::new(store))
            .listening(None)
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
    let project = scratch.project("shop", &single());

    let racers: Vec<JoinHandle<Output>> = (0..RACERS)
        .map(|_| {
            let scratch = Arc::clone(&scratch);
            let project = project.clone();
            thread::spawn(move || scratch.on(&project, &["ps", "--json"]))
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
    let project = scratch.project("shop", &single());
    let old = in_process_service(&scratch, OLD_VERSION, false);
    assert!(matches!(
        scratch.send(&ServiceRequest::Up {
            project: root(&project),
            environment: LaunchEnvironment::default(),
            wait: None,
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

    let ps = scratch.on(&project, &["ps"]);
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
    let project = scratch.project("shop", &single());
    let old = in_process_service(&scratch, OLD_VERSION, true);
    let mut pid = None;
    scratch.send(&ServiceRequest::Up {
        project: root(&project),
        environment: LaunchEnvironment::default(),
        wait: None,
    });
    wait_until("the held launch to run api", || {
        let (state, running) = scratch.first_row_raw(&project);
        pid = running.filter(|_| state == "running");
        pid.is_some()
    });
    scratch.children.push(pid.unwrap() as u32);
    scratch.send(&ServiceRequest::Control {
        project: root(&project),
        name: ProcessName::try_new("api").unwrap(),
        action: ProcessAction::Restart,
        environment: LaunchEnvironment::default(),
        wait: None,
    });
    wait_until("the restart's launch to be held", || {
        scratch.first_row_raw(&project).0 == "starting"
    });

    let mut replacing: Child = scratch
        .command(&["-p", project.to_str().unwrap(), "ps"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    wait_until("the old service to start making way", || {
        !protocol::socket_path(&scratch.state_dir()).exists()
            || replacing.try_wait().unwrap().is_some()
    });
    let second = scratch.on(&project, &["ps", "--json"]);

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
    let project = scratch.project("shop", &single());
    let newer = in_process_service(&scratch, NEWER_VERSION, false);

    let ps = scratch.on(&project, &["ps"]);

    assert_eq!(
        ps.status.code(),
        Some(SERVICE_UNAVAILABLE),
        "{}",
        shown(&ps)
    );
    let stderr = stderr(&ps);
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
    let project = scratch.project("shop", &shop());

    let (output, report) = scratch.json_on(&project, &["start", "dev server"]);
    assert!(output.status.success(), "{}", shown(&output));
    assert_eq!(report["processes"][0]["name"], "dev server");
    assert_eq!(report["processes"][0]["outcome"], "started");

    let (output, report) = scratch.json_on(&project, &["start", "cafe\u{301}"]);
    assert!(output.status.success(), "{}", shown(&output));
    assert_eq!(report["processes"][0]["name"], "caf\u{e9}");

    let unknown = scratch.on(&project, &["stop", "dev"]);
    assert_eq!(unknown.status.code(), Some(INVALID_INPUT));
    let text = stderr(&unknown);
    assert!(
        text.contains("no process 'dev' in shop")
            && text.contains("valid names: 'dev server', ")
            && text.contains("api"),
        "{text}"
    );
    let (unknown, failure) = scratch.json_on(&project, &["stop", "dev"]);
    assert_eq!(unknown.status.code(), Some(INVALID_INPUT));
    assert_eq!(failure["error"]["kind"], "unknown-process");
    assert_eq!(
        failure["error"]["candidates"],
        serde_json::json!(["dev server", "café", "api"])
    );

    let (output, report) = scratch.json_on(&project, &["stop", "dev server"]);
    assert!(output.status.success(), "{}", shown(&output));
    assert_eq!(report["processes"][0]["outcome"], "stopped");
}

/// `up` reports what the service started, `ps` lists them under the
/// project's name, `logs` prints what they wrote, `down` stops them; a crash
/// is reported with its exit, fails the command, and its log stays readable
/// after it exited. Stopping the crashed process leaves it crashed.
#[test]
fn up_ps_logs_and_down_report_what_happened() {
    let scratch = Scratch::new();
    let project = scratch.project("shop", &shop());

    let up = scratch.on(&project, &["up"]);
    assert!(up.status.success(), "{}", shown(&up));
    let text = stdout(&up);
    assert!(
        text.contains("dev server") && text.contains("running (pid"),
        "{text}"
    );
    assert!(!text.contains("api"), "api does not autostart: {text}");

    let again = scratch.on(&project, &["up"]);
    assert!(
        stdout(&again).contains("nothing to start"),
        "{}",
        shown(&again)
    );

    let stable = |output: &Output| -> Vec<String> {
        let text = stdout(output);
        let header = text.lines().nth(1).unwrap_or_default().to_string();
        let width = header.find("UPTIME").unwrap_or(header.len());
        text.lines()
            .map(|line| line.chars().take(width).collect::<String>())
            .collect()
    };
    let ps = scratch.on(&project, &["ps"]);
    let listing = stdout(&ps);
    assert!(
        listing.starts_with(&format!("shop  {}", project.display())),
        "plain ps names its project: {}",
        shown(&ps)
    );
    assert!(
        listing
            .lines()
            .nth(1)
            .unwrap_or_default()
            .starts_with("NAME"),
        "{listing}"
    );
    assert_eq!(
        stable(&ps),
        stable(&scratch.on(&project, &[])),
        "bare muster prints ps"
    );

    wait_until("the dev server to write", || {
        stdout(&scratch.on(&project, &["logs", "dev server"])).contains("dev-ready")
    });

    let (down, report) = scratch.json_on(&project, &["down"]);
    assert!(down.status.success(), "{}", shown(&down));
    let outcomes: Vec<&str> = report["processes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|process| process["outcome"].as_str())
        .collect();
    assert_eq!(outcomes, vec!["stopped", "stopped"], "{report}");
    assert!(
        stdout(&scratch.on(&project, &["logs", "dev server"])).contains("dev-ready"),
        "a stopped process's log is still read"
    );
    let (grepped, slice) = scratch.json_on(&project, &[
        "logs",
        "dev server",
        "--grep",
        "dev-r.ady",
        "--regex",
    ]);
    assert!(grepped.status.success(), "{}", shown(&grepped));
    assert_eq!(slice["text"], "dev-ready", "{slice}");
    let cursor = slice["cursor"].as_str().unwrap();
    let (_, after) = scratch.json_on(&project, &["logs", "dev server", "--since", cursor]);
    assert_eq!(after["lines_returned"], 0, "{after}");
    let bad = scratch.on(&project, &["logs", "dev server", "--grep", "(", "--regex"]);
    assert_eq!(bad.status.code(), Some(INVALID_INPUT), "{}", shown(&bad));

    let broken = scratch.project("broken", &failing());
    let up = scratch.on(&broken, &["up"]);
    assert_eq!(up.status.code(), Some(ACTION_FAILED), "{}", shown(&up));
    let text = stdout(&up);
    assert!(
        text.contains("broken") && text.contains("crashed: exited with status 3"),
        "{text}"
    );
    assert!(text.contains("web") && text.contains("running"), "{text}");
    let row = scratch.row(&broken, "broken");
    assert_eq!(row["state"], "crashed");
    assert_eq!(row["last_exit"]["status"], 3);
    assert!(
        stdout(&scratch.on(&broken, &["logs", "broken"])).contains("oops"),
        "a crashed process's log is read after the fact"
    );
    let (_, notices) = scratch.json_on(&broken, &["notices"]);
    assert!(
        notices.as_array().unwrap().iter().any(|notice| {
            notice["process"] == "broken" && notice["message"] == "crashed: exited with status 3"
        }),
        "{notices}"
    );

    for action in ["stop", "kill"] {
        let (output, report) = scratch.json_on(&broken, &[action, "broken"]);
        assert!(output.status.success(), "{action}: {}", shown(&output));
        assert_eq!(report["processes"][0]["outcome"], "was-not-running");
        assert_eq!(
            scratch.row(&broken, "broken")["state"],
            "crashed",
            "{action} keeps a crashed process crashed"
        );
    }
}

/// A neighbour crash-looping while the command runs is context, not a
/// reason for the command to fail.
#[test]
fn a_crash_looping_neighbour_does_not_fail_a_stop() {
    let scratch = Scratch::new();
    let project = scratch.project("shop", &crash_loop());
    scratch.on(&project, &["up"]);
    wait_until("api to run", || {
        scratch.row(&project, "api")["state"] == "running"
    });

    let (stop, report) = scratch.json_on(&project, &["stop", "api"]);

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
    scratch.on(&project, &["stop", "flaky"]);
}

/// A stop waits out the process's configured grace and the kill after it,
/// and reports the process stopped.
#[test]
fn a_stop_waits_for_a_slow_process() {
    let scratch = Scratch::new();
    let project = scratch.project("shop", &slow_stop());
    scratch.on(&project, &["up"]);
    wait_until("slow to run", || {
        scratch.row(&project, "slow")["state"] == "running"
    });

    let stop = scratch.on(&project, &["stop", "slow"]);

    assert!(stop.status.success(), "{}", shown(&stop));
    assert!(stdout(&stop).contains("slow: stopped"), "{}", shown(&stop));
    assert_eq!(scratch.row(&project, "slow")["stop_grace"], 1);
}

/// `logs -f` of a stopped process shows its log and then follows it when it
/// starts again; a reader that goes away ends it quietly. Plain `logs` into
/// a pipe writes the log byte for byte.
#[test]
fn logs_follow_waits_for_a_stopped_process_and_a_closed_pipe_ends_it() {
    let scratch = Scratch::new();
    let project = scratch.project("shop", &shop());

    let mut follow = scratch
        .command(&["-p", project.to_str().unwrap(), "logs", "-f", "api"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let (lines, received) = unbounded();
    let mut out = BufReader::new(follow.stdout.take().unwrap());
    let reader = thread::spawn(move || {
        let mut line = String::new();
        while out.read_line(&mut line).is_ok_and(|read| read > 0) {
            let _ = lines.send(line.clone());
            line.clear();
        }
    });
    let start = scratch.on(&project, &["start", "api"]);
    assert!(start.status.success(), "{}", shown(&start));
    let got = received.recv_timeout(WAIT).unwrap();
    assert!(got.contains("api-ran"), "{got}");
    let _ = follow.kill();
    let _ = follow.wait();
    reader.join().unwrap();
    let piped = scratch.on(&project, &["logs", "api"]);
    assert!(
        piped
            .stdout
            .windows(b"api-ran".len())
            .any(|window| window == b"api-ran"),
        "{}",
        shown(&piped)
    );

    let ticker = scratch.project("tick", &ticking());
    scratch.on(&ticker, &["up"]);
    let mut follow = scratch
        .command(&["-p", ticker.to_str().unwrap(), "logs", "-f", "ticker"])
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
    let mut errors = String::new();
    follow
        .stderr
        .take()
        .unwrap()
        .read_to_string(&mut errors)
        .unwrap();
    assert_eq!(ended.unwrap().code(), Some(0), "{errors}");
    assert!(errors.is_empty(), "{errors}");
}

/// `muster run` adds the command to the project and starts it under the
/// service with the environment it was run in; bad arguments and a taken
/// name fail before anything is written or started.
#[test]
fn run_adds_the_command_and_starts_it_under_the_service() {
    let scratch = Scratch::new();
    let project = scratch.project("shop", &single());

    let blank = scratch.on(&project, &["run", "--name", " ", "--", " "]);
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

    let output = scratch
        .command(&[
            "-p",
            project.to_str().unwrap(),
            "run",
            "--json",
            "--name",
            "tail job",
            "--",
            &format!("echo caller-${CALLER_VARIABLE}; sleep 600"),
        ])
        .env(CALLER_VARIABLE, "seen")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", shown(&output));
    let report = parsed(&output);
    assert_eq!(report["action"], "run");
    assert_eq!(report["processes"][0]["outcome"], "started");
    assert!(scratch.exported(&project).contains("tail job"));
    assert_eq!(scratch.row(&project, "tail job")["state"], "running");
    wait_until("the run to see its caller's environment", || {
        stdout(&scratch.on(&project, &["logs", "tail job"])).contains("caller-seen")
    });

    let first = scratch.on(&project, &["run", "--", "sleep", "600"]);
    assert!(first.status.success(), "{}", shown(&first));
    let before = scratch.exported(&project);
    let second = scratch.on(&project, &["run", "--", "sleep", "601"]);
    assert_eq!(
        second.status.code(),
        Some(INVALID_INPUT),
        "{}",
        shown(&second)
    );
    let text = stderr(&second);
    assert!(
        text.contains("a process named 'sleep' already exists in shop")
            && text.contains("pass --name to choose another"),
        "the default name is refused with a hint: {text}"
    );
    assert_eq!(scratch.exported(&project), before, "nothing was written");
}

/// The life of a process from the command line: `add` registers the folder
/// and stores it, `up` starts it, `set` changes it while it keeps running,
/// `rm` stops and removes it, and the project stays registered.
#[test]
fn add_up_set_rm_against_a_real_service() {
    let scratch = Scratch::new();
    let folder = scratch.path().join("shop");
    fs::create_dir_all(folder.join("frontend")).unwrap();
    let dir = folder.to_str().unwrap();

    let add = scratch.muster(&[
        "-p",
        dir,
        "add",
        "web",
        "--autostart",
        "--cwd",
        folder.join("frontend").to_str().unwrap(),
        "--env",
        "PORT=5173",
        "--json",
        "--",
        "echo port-$PORT; sleep 600",
    ]);
    assert!(add.status.success(), "{}", shown(&add));
    let added = parsed(&add);
    assert_eq!(added["process"]["command"], "echo port-$PORT; sleep 600");
    assert_eq!(added["registered"], true);
    assert_eq!(added["project"]["name"], "shop");
    assert_eq!(added["project"]["root"], dir);
    assert_eq!(added["process"]["working_dir"], "frontend");
    assert_eq!(added["process"]["environment"]["PORT"], "5173");
    assert_eq!(added["process"]["autostart"], true);
    assert_eq!(added["process"]["restart"], "never", "the default setting");
    assert!(
        processes_under(SERVICE_SUBCOMMAND, scratch.root.path()).is_empty(),
        "add starts nothing"
    );
    let taken = scratch.muster(&["-p", dir, "add", "web", "--", "true"]);
    assert_eq!(
        taken.status.code(),
        Some(INVALID_INPUT),
        "{}",
        shown(&taken)
    );
    for refused in ["we\u{1b}]0;x\u{7}b", "line\nbreak"] {
        let added = scratch.muster(&["-p", dir, "add", refused, "--", "true"]);
        assert_eq!(
            added.status.code(),
            Some(INVALID_INPUT),
            "a name with a control character is refused: {}",
            shown(&added)
        );
    }
    let env = scratch.muster(&["-p", dir, "add", "x", "--env", "NOEQUALS", "--", "true"]);
    assert_eq!(env.status.code(), Some(INVALID_INPUT), "{}", shown(&env));

    let up = scratch.on(&folder, &["up"]);
    assert!(up.status.success(), "{}", shown(&up));
    wait_until("web to run in its folder with its variable", || {
        stdout(&scratch.on(&folder, &["logs", "web"])).contains("port-5173")
    });
    let pid = scratch.row(&folder, "web")["pid"].clone();

    let nothing = scratch.on(&folder, &["set", "web"]);
    assert_eq!(nothing.status.code(), Some(USAGE), "{}", shown(&nothing));
    let (set, changed) = scratch.json_on(&folder, &["set", "web", "--restart", "on-failure"]);
    assert!(set.status.success(), "{}", shown(&set));
    assert_eq!(changed["process"]["restart"], "on-failure");
    assert_eq!(changed["running"], true);
    assert_eq!(
        scratch.row(&folder, "web")["pid"],
        pid,
        "a change waits for the next start"
    );
    let human = scratch.on(&folder, &["set", "web", "--description", "the site"]);
    assert!(
        stdout(&human).contains("`muster restart 'web'` applies the change"),
        "{}",
        shown(&human)
    );
    let (ready, with_ready) = scratch.json_on(&folder, &[
        "set",
        "web",
        "--ready-log",
        "ready in \\d+ms",
        "--ready-regex",
        "--ready-timeout",
        "90s",
    ]);
    assert!(ready.status.success(), "{}", shown(&ready));
    assert_eq!(
        with_ready["process"]["ready"],
        serde_json::json!({ "log": "ready in \\d+ms", "regex": true, "timeout": "1m 30s" })
    );
    let exported = stdout(&scratch.on(&folder, &["export", "-"]));
    assert!(
        exported.contains("ready:") && exported.contains("regex: true"),
        "{exported}"
    );
    let timeless = scratch.on(&folder, &["set", "web", "--ready-port", "3000"]);
    assert_eq!(timeless.status.code(), Some(USAGE), "{}", shown(&timeless));
    let bad_regex = scratch.on(&folder, &[
        "set",
        "web",
        "--ready-log",
        "(",
        "--ready-regex",
        "--ready-timeout",
        "5s",
    ]);
    assert_eq!(
        bad_regex.status.code(),
        Some(INVALID_INPUT),
        "{}",
        shown(&bad_regex)
    );
    let (_, cleared) = scratch.json_on(&folder, &["set", "web", "--no-ready"]);
    assert_eq!(cleared["process"]["ready"], serde_json::Value::Null);

    let (rm, removed) = scratch.json_on(&folder, &["rm", "web"]);
    assert!(rm.status.success(), "{}", shown(&rm));
    assert_eq!(
        removed["removed"],
        serde_json::json!([{ "name": "web", "stopped": true, "still_stopping": false }])
    );
    let (_, statuses) = scratch.json_on(&folder, &["ps"]);
    assert_eq!(statuses[0]["processes"], serde_json::json!([]));
    let (_, projects) = scratch.json(&["projects"]);
    assert!(
        projects
            .as_array()
            .unwrap()
            .iter()
            .any(|listed| listed["name"] == "shop" && listed["processes"] == 0),
        "the project stays registered: {projects}"
    );
    let unknown = scratch.on(&folder, &["rm", "web"]);
    assert_eq!(
        unknown.status.code(),
        Some(INVALID_INPUT),
        "{}",
        shown(&unknown)
    );
}

/// `config` lists, reads and changes the settings; a bad value is invalid
/// input naming the key, and a misspelt key lists the valid ones.
#[test]
fn config_lists_reads_and_changes_settings() {
    let scratch = Scratch::new();

    let get = scratch.muster(&["config", "get", "default-stop-grace"]);
    assert!(get.status.success(), "{}", shown(&get));
    assert_eq!(stdout(&get), "5s\n");
    let (_, listed) = scratch.json(&["config"]);
    let settings = listed["settings"].as_array().unwrap();
    assert!(settings.contains(&serde_json::json!({
        "key": "desktop-notifications",
        "value": "false",
    })));
    assert!(settings.contains(&serde_json::json!({
        "key": "default-stop-grace",
        "value": "5s",
    })));

    let (set, changed) = scratch.json(&["config", "set", "detach-key", "ctrl-]"]);
    assert!(set.status.success(), "{}", shown(&set));
    assert_eq!(
        changed,
        serde_json::json!({ "key": "detach-key", "value": "ctrl-]" })
    );
    let (_, value) = scratch.json(&["config", "get", "detach-key"]);
    assert_eq!(value["value"], "ctrl-]");

    let bad = scratch.muster(&["config", "set", "default-size", "0x80"]);
    assert_eq!(bad.status.code(), Some(INVALID_INPUT), "{}", shown(&bad));
    for typed_by_terminals in ["ctrl-m", "ctrl-j", "ctrl-i", "ctrl-h", "ctrl-c"] {
        let refused = scratch.muster(&["config", "set", "detach-key", typed_by_terminals]);
        assert_eq!(
            refused.status.code(),
            Some(INVALID_INPUT),
            "{typed_by_terminals} is a key terminals send for Enter, Tab, Backspace or interrupt: {}",
            shown(&refused)
        );
    }
    let typo = scratch.muster(&["config", "get", "detach_key"]);
    assert_eq!(typo.status.code(), Some(USAGE), "{}", shown(&typo));
    assert!(
        stderr(&typo).contains("default-stop-grace"),
        "{}",
        shown(&typo)
    );
}

/// A stack exported from one project imports into another folder,
/// registering it; a strict import of names already there refuses and
/// writes nothing, `--merge` updates them and `--replace` removes the rest.
#[test]
fn export_and_import_move_a_stack_between_projects() {
    let scratch = Scratch::new();
    let shop = scratch.project("shop", &shop());
    let file = scratch.path().join("stack.yml");

    let (export, written) = scratch.json_on(&shop, &["export", file.to_str().unwrap()]);
    assert!(export.status.success(), "{}", shown(&export));
    assert_eq!(written["processes"], 3);
    let again = scratch.on(&shop, &["export", file.to_str().unwrap()]);
    assert_eq!(
        again.status.code(),
        Some(INVALID_INPUT),
        "{}",
        shown(&again)
    );

    let copy = scratch.path().join("copy");
    fs::create_dir_all(&copy).unwrap();
    let (import, imported) = scratch.json_on(&copy, &["import", file.to_str().unwrap()]);
    assert!(import.status.success(), "{}", shown(&import));
    assert_eq!(imported["registered"], true);
    assert_eq!(
        imported["added"],
        serde_json::json!(["dev server", "café", "api"])
    );
    let before = scratch.exported(&copy);

    let conflict = scratch.on(&copy, &["import", file.to_str().unwrap()]);
    assert_eq!(
        conflict.status.code(),
        Some(INVALID_INPUT),
        "{}",
        shown(&conflict)
    );
    assert!(
        stderr(&conflict).contains("--merge"),
        "{}",
        shown(&conflict)
    );
    assert_eq!(
        scratch.exported(&copy),
        before,
        "a refused import writes nothing"
    );

    let smaller = scratch.path().join("smaller.yml");
    fs::write(
        &smaller,
        stack(&[entry("api", "echo changed; sleep 600", false)]),
    )
    .unwrap();
    let (merge, merged) = scratch.json_on(&copy, &["import", smaller.to_str().unwrap(), "--merge"]);
    assert!(merge.status.success(), "{}", shown(&merge));
    assert_eq!(merged["updated"], serde_json::json!(["api"]));
    let merged_stack = scratch.exported(&copy);
    let (dry, planned) = scratch.json_on(&copy, &[
        "import",
        smaller.to_str().unwrap(),
        "--replace",
        "--dry-run",
    ]);
    assert!(dry.status.success(), "{}", shown(&dry));
    assert_eq!(planned["dry_run"], true);
    assert_eq!(planned["removed"].as_array().unwrap().len(), 2, "{planned}");
    assert_eq!(
        scratch.exported(&copy),
        merged_stack,
        "a dry run writes nothing"
    );
    let (replace, replaced) =
        scratch.json_on(&copy, &["import", smaller.to_str().unwrap(), "--replace"]);
    assert!(replace.status.success(), "{}", shown(&replace));
    let removed: Vec<&str> = replaced["removed"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|removed| removed["name"].as_str())
        .collect();
    assert_eq!(removed, ["dev server", "café"]);
    assert!(!scratch.exported(&copy).contains("dev server"));
}

/// `init` reads what the folder offers: listed without a terminal and
/// nothing written, the suggested ones added with `--yes`.
#[test]
fn init_lists_without_a_terminal_and_adds_the_suggested_with_yes() {
    let scratch = Scratch::new();
    let folder = scratch.path().join("site");
    fs::create_dir_all(&folder).unwrap();
    fs::write(
        folder.join("package.json"),
        r#"{"scripts": {"dev": "vite", "build": "vite build"}}"#,
    )
    .unwrap();
    fs::write(folder.join("Procfile"), "worker: bin/worker\n").unwrap();
    let dir = folder.to_str().unwrap();

    let plain = scratch.muster(&["-p", dir, "init"]);
    assert!(plain.status.success(), "{}", shown(&plain));
    let listed = stdout(&plain);
    assert!(
        listed.contains("[x] dev  npm run dev  package-json"),
        "{listed}"
    );
    assert!(
        listed.contains("[ ] build  npm run build  package-json"),
        "{listed}"
    );
    assert!(
        listed.contains("[x] worker  bin/worker  procfile"),
        "{listed}"
    );
    let (_, projects) = scratch.json(&["projects"]);
    assert_eq!(
        projects,
        serde_json::json!([]),
        "a plain init writes nothing"
    );

    let (yes, added) = scratch.json(&["-p", dir, "init", "--yes"]);
    assert!(yes.status.success(), "{}", shown(&yes));
    assert_eq!(added["added"], serde_json::json!(["dev", "worker"]));
    let (_, again) = scratch.json(&["-p", dir, "init"]);
    let offered: Vec<&str> = again["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|candidate| candidate["name"].as_str())
        .collect();
    assert_eq!(
        offered,
        ["build"],
        "names already added are not offered again"
    );
}

/// A command run in a subfolder acts on the deepest project containing it,
/// never on some other registered project; outside every project it is
/// refused before a service starts.
#[test]
fn the_project_is_the_folder_the_command_runs_in() {
    let scratch = Scratch::new();
    let shop = scratch.project("shop", &single());
    let nested = shop.join("frontend");
    fs::create_dir_all(nested.join("src")).unwrap();
    let add = scratch
        .command(&["-p", ".", "add", "vite", "--", "sleep", "600"])
        .current_dir(&nested)
        .output()
        .unwrap();
    assert!(add.status.success(), "{}", shown(&add));

    let names = |cwd: &Path| -> Value {
        let output = scratch
            .command(&["ps", "--json"])
            .current_dir(cwd)
            .output()
            .unwrap();
        parsed(&output)[0]["project"]["name"].clone()
    };
    assert_eq!(names(&shop), "shop");
    assert_eq!(names(&nested.join("src")), "frontend");

    let elsewhere = scratch.path().join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    let outside = scratch
        .command(&["stop", "api"])
        .current_dir(&elsewhere)
        .output()
        .unwrap();
    assert_eq!(
        outside.status.code(),
        Some(INVALID_INPUT),
        "{}",
        shown(&outside)
    );
    assert!(
        stderr(&outside).contains("no muster project in"),
        "{}",
        shown(&outside)
    );
    let unknown = scratch.muster(&["-p", "nosuch", "ps"]);
    assert_eq!(
        unknown.status.code(),
        Some(INVALID_INPUT),
        "{}",
        shown(&unknown)
    );
}

/// `service stop` leaves every process running; the next command brings a
/// service back that adopts them.
#[test]
fn service_stop_keeps_processes_and_the_next_command_restarts_it() {
    let scratch = Scratch::new();
    let project = scratch.project("shop", &single());
    scratch.on(&project, &["up"]);
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

/// A service that cannot start, here for a database a newer muster wrote,
/// makes the command fail with the reason from this start's log lines only.
#[test]
fn a_service_that_cannot_start_shows_why() {
    let scratch = Scratch::new();
    fs::write(
        protocol::log_path(&scratch.state_dir()),
        "an older failure from another day\n",
    )
    .unwrap();
    rusqlite::Connection::open(scratch.database())
        .unwrap()
        .pragma_update(None, "user_version", FUTURE_SCHEMA)
        .unwrap();

    let ps = scratch.muster(&["ps", "--all"]);

    assert_eq!(
        ps.status.code(),
        Some(SERVICE_UNAVAILABLE),
        "{}",
        shown(&ps)
    );
    let text = stderr(&ps);
    assert!(
        text.contains("the muster service is not available"),
        "{text}"
    );
    assert!(text.contains("upgrade muster"), "{text}");
    assert!(
        !text.contains("older failure"),
        "only this start's lines: {text}"
    );
}

/// `project` listing one process, `api`, that has not started.
fn pending_api_status(project: &Path) -> ProjectStatus {
    ProjectStatus::builder()
        .project(
            Project::builder()
                .root(root(project))
                .name(ProjectName::try_new("shop").unwrap())
                .build(),
        )
        .processes(vec![
            ProcessStatus::builder()
                .name(ProcessName::try_new("api").unwrap())
                .state(ProcessState::Pending)
                .pid(None)
                .uptime(None)
                .restarts(0)
                .usage(None)
                .attached(0)
                .last_exit(None)
                .stop_grace(Duration::from_secs(5))
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
                        .project(Some(root(&project)))
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
    let project = scratch.project("shop", &single());
    let reason = REFUSAL_REASON.to_string();
    scripted_service(&scratch, project.clone());

    let (up, report) = scratch.json_on(&project, &["up"]);
    let human = scratch.on(&project, &["up"]);

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
    assert_eq!(
        stdout(&human).matches(reason.as_str()).count(),
        1,
        "{}",
        shown(&human)
    );
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

/// Commands that need a project, an existing name or a confirmation exit
/// with the codes the help documents, before any service starts.
#[test]
fn input_errors_fail_before_the_service_starts() {
    let scratch = Scratch::new();

    let outside = scratch.muster(&["ps"]);
    assert_eq!(
        outside.status.code(),
        Some(INVALID_INPUT),
        "{}",
        shown(&outside)
    );
    assert!(
        stderr(&outside).contains("no muster project in"),
        "{}",
        shown(&outside)
    );
    let remove = scratch.muster(&["projects", "rm", "nosuch", "--yes"]);
    assert_eq!(
        remove.status.code(),
        Some(INVALID_INPUT),
        "{}",
        shown(&remove)
    );
    let project = scratch.project("shop", &single());
    let unconfirmed = scratch.muster(&["projects", "rm", "shop"]);
    assert_eq!(
        unconfirmed.status.code(),
        Some(USAGE),
        "without a terminal --yes is needed: {}",
        shown(&unconfirmed)
    );
    let duplicate = scratch.path().join("dup.yml");
    fs::write(
        &duplicate,
        stack(&[
            entry("api", "sleep 1", false),
            entry("api", "sleep 2", false),
        ]),
    )
    .unwrap();
    let import = scratch.on(&project, &[
        "import",
        duplicate.to_str().unwrap(),
        "--merge",
    ]);
    assert_eq!(
        import.status.code(),
        Some(INVALID_INPUT),
        "{}",
        shown(&import)
    );
    assert!(
        processes_under(SERVICE_SUBCOMMAND, scratch.root.path()).is_empty(),
        "no service was started"
    );

    let usage = scratch.muster(&["start", "--bogus", "--json"]);
    assert_eq!(usage.status.code(), Some(USAGE));
    assert_eq!(parsed(&usage)["error"]["kind"], "usage");
}

/// `projects rm` stops the project's processes and forgets it with them.
#[test]
fn projects_rm_stops_and_forgets_the_project() {
    let scratch = Scratch::new();
    let project = scratch.project("shop", &single());
    scratch.on(&project, &["up"]);
    let pid = scratch.row(&project, "api")["pid"].as_u64().unwrap() as u32;

    let (rm, removal) = scratch.json(&["projects", "rm", "shop", "--yes"]);

    assert!(rm.status.success(), "{}", shown(&rm));
    assert_eq!(removal["removed"], true);
    assert_eq!(removal["stopped"]["processes"][0]["outcome"], "stopped");
    wait_until("the process to end", || !alive(pid));
    let (_, projects) = scratch.json(&["projects"]);
    assert_eq!(projects, serde_json::json!([]));
}

/// The MCP protocol version the test client speaks.
const MCP_PROTOCOL_VERSION: &str = "2025-06-18";
/// The JSON-RPC version every MCP message carries.
const JSON_RPC_VERSION: &str = "2.0";
/// The tools that act on one process, each of which takes its `name`.
const PROCESS_TOOLS: [&str; 7] = [
    "process_screen",
    "process_logs",
    "process_wait",
    "process_send",
    "process_start",
    "process_stop",
    "process_restart",
];

/// A project for the MCP tools: a `cat` that echoes what it is sent, and a
/// runner with an accent in its name. Neither autostarts.
fn mcp_project() -> String {
    stack(&[
        entry("echo box", "cat", false),
        entry("café", "echo café-ready; sleep 600", false),
    ])
}

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
            .command(&["-p", project.to_str().unwrap(), "mcp", "--as", "test-agent"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        scratch.children.push(child.id());
        let out = child.stdout.take().unwrap();
        let (sender, lines) = unbounded();
        thread::spawn(move || {
            for line in BufReader::new(out).lines() {
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
fn mcp_tools_work_with_the_service_autostarted() {
    let mut scratch = Scratch::new();
    let project = scratch.project("shop", &mcp_project());
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
        assert!(
            schema["properties"]["project"].is_object(),
            "{tool}: {schema}"
        );
        assert!(schema["properties"]["id"].is_null(), "{tool}: {schema}");
    }

    let listed = mcp.result("stack_status", serde_json::json!({}))["structuredContent"].clone();
    assert_eq!(listed["project"]["name"], "shop", "{listed}");
    assert_eq!(listed["you"], "cli-exec@test-agent", "{listed}");
    assert!(listed["alerts"].is_array(), "{listed}");
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
    assert_eq!(report["process"], "echo box");
    assert_eq!(report["outcome"], "started");
    assert_eq!(report["status"]["state"], "running");

    let sent = mcp.result(
        "process_send",
        serde_json::json!({ "name": "echo box", "text": "hello muster", "enter": true }),
    );
    assert_eq!(sent["structuredContent"]["sent"], true, "{sent}");
    wait_until("the echo to reach the screen", || {
        mcp.text("process_screen", serde_json::json!({ "name": "echo box" }))
            .contains("hello muster")
    });

    let accented = mcp.result(
        "process_start",
        serde_json::json!({ "name": "cafe\u{301}", "project": "shop" }),
    );
    assert_eq!(accented["structuredContent"]["process"], "caf\u{e9}");

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
    assert_eq!(restarted["structuredContent"]["outcome"], "restarted");
    for name in ["echo box", "café"] {
        let stopped = mcp.result("process_stop", serde_json::json!({ "name": name }));
        assert_eq!(
            stopped["structuredContent"]["outcome"], "stopped",
            "{stopped}"
        );
    }
    assert_eq!(
        mcp.result("stack_status", serde_json::json!({}))["structuredContent"]["processes"][1]["last_exit"]
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

/// The agent's loop against a real service: a stored readiness check gates
/// a start with wait=true, a restart's wait sees only the new run, the log
/// reads the current run from `run_start` and nothing after its own cursor,
/// a wait that runs out of time is not an error, and a crash while waiting
/// is, carrying its last lines, an alert and an event.
#[test]
fn mcp_waits_reads_logs_and_hears_crashes() {
    let mut scratch = Scratch::new();
    let project = scratch.project(
        "shop",
        &stack(&[
            entry("api", "echo booting; echo ready on $$; sleep 600", false),
            entry("flaky", "echo boom; exit 3", false),
        ]),
    );
    let mut mcp = McpSession::start(&mut scratch, &project);

    let set = mcp.result(
        "process_set",
        serde_json::json!({ "name": "api", "ready": { "log": "ready on", "timeout": "30s" } }),
    );
    assert_eq!(
        set["structuredContent"]["process"]["ready"],
        serde_json::json!({ "log": "ready on", "timeout": "30s" }),
        "{set}"
    );
    let started = mcp.result(
        "process_start",
        serde_json::json!({ "name": "api", "wait": true }),
    );
    assert_eq!(started["isError"], false, "{started}");
    let wait = &started["structuredContent"]["wait"];
    assert_eq!(wait["status"], "matched", "{started}");
    assert_eq!(wait["until"], serde_json::json!({ "ready": true }));
    let run_start = |mcp: &mut McpSession| {
        let status = mcp.result("stack_status", serde_json::json!({}))["structuredContent"].clone();
        assert_eq!(status["processes"][0]["ready"], "ready", "{status}");
        status["processes"][0]["run_start"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let first_run = run_start(&mut mcp);

    let restarted = mcp.result(
        "process_restart",
        serde_json::json!({ "name": "api", "wait": { "log": "ready on" } }),
    );
    assert_eq!(
        restarted["structuredContent"]["wait"]["status"], "matched",
        "{restarted}"
    );
    let second_run = run_start(&mut mcp);
    assert_ne!(first_run, second_run);
    let logs = mcp.result(
        "process_logs",
        serde_json::json!({ "name": "api", "since": second_run }),
    );
    let slice = &logs["structuredContent"];
    assert_eq!(slice["lines_returned"], 2, "{logs}");
    assert!(
        slice["text"]
            .as_str()
            .unwrap()
            .starts_with("booting\nready on"),
        "{logs}"
    );
    assert!(result_text(&logs).starts_with("api o:"), "{logs}");
    let after = mcp.result(
        "process_logs",
        serde_json::json!({ "name": "api", "since": slice["cursor"] }),
    );
    assert_eq!(after["structuredContent"]["lines_returned"], 0, "{after}");

    let waiting = mcp.result(
        "process_wait",
        serde_json::json!({ "name": "api", "until": { "log": "never printed" }, "timeout_s": 1 }),
    );
    assert_eq!(waiting["isError"], false, "{waiting}");
    assert_eq!(waiting["structuredContent"]["status"], "waiting");
    assert!(
        waiting["structuredContent"]["hint"]
            .as_str()
            .unwrap()
            .contains("since=o:"),
        "{waiting}"
    );

    let crashed = mcp.result(
        "process_start",
        serde_json::json!({ "name": "flaky", "wait": { "log": "never printed" } }),
    );
    assert_eq!(crashed["isError"], true, "{crashed}");
    assert_eq!(crashed["structuredContent"]["wait"]["status"], "crashed");
    assert!(result_text(&crashed).contains("boom"), "{crashed}");
    let alerted = |result: &Value| {
        result["structuredContent"]["alerts"]
            .as_array()
            .is_some_and(|alerts| alerts.iter().any(|alert| alert["kind"] == "crashed"))
    };
    let mut heard = alerted(&crashed);
    wait_until("the crash to arrive as an alert", || {
        heard = heard || alerted(&mcp.result("stack_status", serde_json::json!({})));
        heard
    });
    let events = mcp.result(
        "stack_events",
        serde_json::json!({ "kinds": ["crashed", "ready"] }),
    )["structuredContent"]
        .clone();
    let kinds: Vec<&str> = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["kind"].as_str().unwrap())
        .collect();
    assert!(
        kinds.contains(&"crashed") && kinds.contains(&"ready"),
        "{events}"
    );
    mcp.result("process_stop", serde_json::json!({ "name": "api" }));
}

/// An agent's MCP server in a folder that is not yet a project registers it
/// with `process_add`, sees a running process through `process_set`, and
/// `process_remove` stops it through the service before answering; the
/// launch carries the server's own environment.
#[test]
fn mcp_edits_the_stack_of_a_new_folder_against_a_real_service() {
    let mut scratch = Scratch::new();
    let folder = scratch.path().join("fresh");
    fs::create_dir_all(&folder).unwrap();
    let mut mcp = McpSession::start(&mut scratch, &folder);

    let outside = mcp.mistake("stack_status", serde_json::json!({}));
    assert!(result_text(&outside).contains("process_add"), "{outside}");
    assert_eq!(
        outside["structuredContent"]["error"]["kind"],
        "unknown-project"
    );
    assert!(
        processes_under(SERVICE_SUBCOMMAND, scratch.root.path()).is_empty(),
        "a folder that is not a project starts no service"
    );

    let added = mcp.result(
        "process_add",
        serde_json::json!({ "name": "web", "command": "echo home-$HOME; sleep 600" }),
    );
    assert_eq!(
        added["structuredContent"]["process"]["name"], "web",
        "{added}"
    );
    assert_eq!(
        added["structuredContent"]["project"]["name"], "fresh",
        "{added}"
    );
    let started = mcp.result("process_start", serde_json::json!({ "name": "web" }));
    assert_eq!(
        started["structuredContent"]["outcome"], "started",
        "{started}"
    );
    let pid = started["structuredContent"]["status"]["pid"]
        .as_u64()
        .unwrap();
    scratch.children.push(pid as u32);
    wait_until("the launch to carry the server's environment", || {
        mcp.text("process_screen", serde_json::json!({ "name": "web" }))
            .contains("home-/")
    });

    let changed = mcp.result(
        "process_set",
        serde_json::json!({ "name": "web", "restart": "on-failure" }),
    );
    assert_eq!(changed["structuredContent"]["running"], true, "{changed}");

    let removed = mcp.result("process_remove", serde_json::json!({ "name": "web" }));
    let removed = &removed["structuredContent"];
    assert_eq!(removed["name"], "web", "{removed}");
    assert_eq!(removed["stopped"], true, "{removed}");
    assert_eq!(removed["still_stopping"], false, "{removed}");
    assert!(!alive(pid as u32), "the removed process was stopped");
    let listed = mcp.result("stack_status", serde_json::json!({}));
    assert_eq!(
        listed["structuredContent"]["processes"],
        serde_json::json!([])
    );

    let stop = scratch.muster(&["service", "stop"]);
    assert!(stop.status.success(), "{}", shown(&stop));
}

/// `mcp --print-config` prints the snippet alone, and a reader that closes
/// early ends it quietly.
#[test]
fn mcp_print_config_is_plain_data() {
    let scratch = Scratch::new();
    let project = scratch.project("shop", &single());

    let printed = scratch.on(&project, &["mcp", "--print-config"]);
    assert!(printed.status.success(), "{}", shown(&printed));
    let snippet = parsed(&printed);
    assert!(snippet["mcpServers"].is_object(), "{snippet}");

    let mut closed = scratch
        .command(&["-p", project.to_str().unwrap(), "mcp", "--print-config"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(closed.stdout.take());
    let output = closed.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(0), "{}", shown(&output));
    assert!(output.stderr.is_empty(), "{}", shown(&output));
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
/// The default detach key, `ctrl-\`.
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

/// Without a terminal to ask on, a command that needs a name fails as
/// invalid input naming what to pass, also in JSON, and changes nothing; a
/// missing name is refused before the service is reached.
#[test]
fn a_question_nobody_can_answer_names_the_argument_to_pass() {
    let scratch = Scratch::new();
    let project = scratch.project("shop", &single());
    let unnamed = scratch.on(&project, &["stop"]);
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
    let up = scratch.on(&project, &["up"]);
    assert!(up.status.success(), "{}", shown(&up));

    for command in ["start", "stop", "restart", "logs"] {
        let output = scratch.on(&project, &[command]);
        assert_eq!(
            output.status.code(),
            Some(INVALID_INPUT),
            "{command}: {}",
            shown(&output)
        );
        assert!(
            stderr(&output).contains("pass its name"),
            "{command}: {}",
            shown(&output)
        );
    }
    let (output, failure) = scratch.json_on(&project, &["stop"]);
    assert_eq!(output.status.code(), Some(INVALID_INPUT));
    assert_eq!(failure["error"]["kind"], "invalid-input", "{failure}");
    assert_eq!(scratch.row(&project, "api")["state"], "running");
}

/// Ctrl-C under the spinner still closes the frame after the process
/// chooser asked its question in it.
#[test]
fn ctrl_c_under_the_spinner_after_a_question_closes_the_frame() {
    let mut scratch = Scratch::new();
    let project = scratch.project(
        "shop",
        &stack(&[entry("api", "sleep 600", true), Entry {
            grace: "1s",
            ..entry("slow", "trap '' TERM; while :; do sleep 0.1; done", true)
        }]),
    );
    let up = scratch.on(&project, &["up"]);
    assert!(up.status.success(), "{}", shown(&up));

    let pty = Pty::open();
    let mut stop = pty.spawn_controlling(&mut scratch, &project, &["stop"]);
    pty.wait_for("Which process?", 1);
    pty.type_keys(ARROW_DOWN);
    pty.type_keys(ENTER);
    pty.wait_for("stopping", 1);
    pty.interrupt();
    assert_eq!(
        Pty::exit_code(&mut stop),
        Some(INTERRUPTED),
        "{}",
        pty.drawn()
    );
    assert!(pty.drawn().contains("└  interrupted"), "{}", pty.drawn());
    let raw = pty.raw();
    assert!(
        raw.rfind("\x1b[?25h") >= raw.rfind("\x1b[?25l"),
        "the cursor is shown again: {raw:?}"
    );
    wait_until("slow to stop", || {
        scratch.row(&project, "slow")["state"] != "running"
    });
}

/// Ctrl-C before the command sent its request cancels it: nothing is sent,
/// and the frame says nothing was changed.
#[test]
fn ctrl_c_before_the_request_is_sent_cancels_it() {
    let mut scratch = Scratch::new();
    let project = scratch.project("shop", &single());
    let asked = slow_status_service(&scratch, project.clone());

    let pty = Pty::open();
    let mut up = pty.spawn_controlling(&mut scratch, &project, &["up"]);
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
    let project = scratch.project("shop", &single());
    scripted_service(&scratch, project.clone());

    let pty = Pty::open();
    let mut logs = pty.spawn(&mut scratch, &project, &["logs"]);
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
/// summary, and `projects rm` asks first: a no removes nothing and
/// succeeds, a yes stops and forgets the project.
#[test]
fn up_is_framed_and_projects_rm_asks_first() {
    let mut scratch = Scratch::new();
    let project = scratch.project(
        "shop",
        &stack(&[
            entry("api", "sleep 600", true),
            entry("web", "sleep 600", true),
        ]),
    );

    let pty = Pty::open();
    let mut up = pty.spawn(&mut scratch, &project, &["up"]);
    assert_eq!(Pty::exit_code(&mut up), Some(0), "{}", pty.drawn());
    let drawn = pty.drawn();
    assert!(drawn.contains("muster up · shop"), "{drawn}");
    assert!(drawn.contains("running (pid"), "{drawn}");
    assert!(drawn.contains("2 ok"), "{drawn}");

    let pty = Pty::open();
    let mut rm = pty.spawn(&mut scratch, &project, &["projects", "rm", "shop"]);
    pty.wait_for("Remove this project", 1);
    pty.type_keys(b"n");
    assert_eq!(Pty::exit_code(&mut rm), Some(0), "{}", pty.drawn());
    assert!(pty.drawn().contains("nothing removed"), "{}", pty.drawn());
    assert_eq!(scratch.row(&project, "api")["state"], "running");

    let pty = Pty::open();
    let mut rm = pty.spawn(&mut scratch, &project, &["projects", "rm", "shop"]);
    pty.wait_for("Remove this project", 1);
    pty.type_keys(b"y");
    assert_eq!(Pty::exit_code(&mut rm), Some(0), "{}", pty.drawn());
    assert!(
        pty.drawn().contains("removed project shop"),
        "{}",
        pty.drawn()
    );
    let (_, projects) = scratch.json(&["projects"]);
    assert_eq!(projects, serde_json::json!([]));
}

/// Ctrl-C while `down` waits under its spinner ends the command at once
/// with the frame closed and exit code 130, and the service finishes the
/// stop on its own.
#[test]
fn ctrl_c_under_the_spinner_closes_the_frame() {
    let mut scratch = Scratch::new();
    let project = scratch.project("shop", &slow_stop());
    let up = scratch.on(&project, &["up"]);
    assert!(up.status.success(), "{}", shown(&up));

    let pty = Pty::open();
    let mut down = pty.spawn_controlling(&mut scratch, &project, &["down"]);
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
    let project = scratch.project("shop", &shop());

    let pty = Pty::open();
    let mut attach = pty.spawn(&mut scratch, &project, &["attach"]);
    assert_eq!(
        Pty::exit_code(&mut attach),
        Some(ACTION_FAILED),
        "{}",
        pty.drawn()
    );
    assert!(pty.drawn().contains("no process"), "{}", pty.drawn());

    let up = scratch.on(&project, &["up"]);
    assert!(up.status.success(), "{}", shown(&up));
    let pty = Pty::open();
    let mut attach = pty.spawn(&mut scratch, &project, &["attach"]);
    pty.wait_for("Which process?", 1);
    let drawn = pty.drawn();
    assert!(drawn.contains("running"), "{drawn}");
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
        project: root(project),
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
    let project = scratch.project("shop", &single());
    let up = scratch.on(&project, &["up"]);
    assert!(up.status.success(), "{}", shown(&up));
    let attached_size = PtySize::builder()
        .rows(Rows::new(ATTACH_ROWS))
        .cols(Cols::new(ATTACH_COLS))
        .build();

    let pty = Pty::open_sized(ATTACH_ROWS, ATTACH_COLS);
    let mut attach = pty.spawn(&mut scratch, &project, &["attach", "api"]);
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
    let project = scratch.project("shop", &shop());
    let up = scratch.on(&project, &["up"]);
    assert!(up.status.success(), "{}", shown(&up));
    wait_until("the dev server to write", || {
        stdout(&scratch.on(&project, &["logs", "dev server"])).contains("dev-ready")
    });

    let pty = Pty::open();
    let mut logs = pty.spawn(&mut scratch, &project, &["logs"]);
    pty.wait_for("Which process?", 1);
    pty.type_keys(ENTER);
    assert_eq!(Pty::exit_code(&mut logs), Some(0), "{}", pty.drawn());
    let drawn = pty.drawn();
    let closed = drawn.rfind("└  dev server").expect(&drawn);
    let log = drawn.rfind("dev-ready").expect(&drawn);
    assert!(closed < log, "{drawn}");
}

/// Data a command prints is never framed, even at a terminal: a note's
/// body, a listing, a setting and the completions line come out raw.
#[test]
fn data_output_is_never_framed() {
    let mut scratch = Scratch::new();
    let project = scratch.project("shop", &single());
    let set = scratch
        .command(&["note", "set", "plan", "ship it"])
        .current_dir(&project)
        .output()
        .unwrap();
    assert!(set.status.success(), "{}", shown(&set));

    for args in [
        &["note", "get", "plan"][..],
        &["note"][..],
        &["config", "get", "detach-key"][..],
        &["projects"][..],
        &["completions", "bash"][..],
    ] {
        let pty = Pty::open();
        let mut child = pty.spawn(&mut scratch, &project, args);
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

/// At a terminal, `init` offers what the folder's files define with the
/// suggested ones chosen, asks whether `muster up` starts them, and adds
/// the choice, registering the folder.
#[test]
fn init_offers_what_the_folder_defines() {
    let mut scratch = Scratch::new();
    let folder = scratch.path().join("fresh");
    fs::create_dir_all(&folder).unwrap();
    fs::write(
        folder.join("package.json"),
        r#"{"scripts": {"dev": "vite", "build": "vite build"}}"#,
    )
    .unwrap();

    let pty = Pty::open();
    let mut init = pty.spawn(&mut scratch, &folder, &["-p", ".", "init"]);
    pty.wait_for("Which processes should muster run?", 1);
    pty.type_keys(ENTER);
    pty.wait_for("Start them with `muster up`?", 1);
    pty.type_keys(b"y");
    assert_eq!(Pty::exit_code(&mut init), Some(0), "{}", pty.drawn());
    assert!(
        pty.drawn()
            .contains("added 1 process; `muster up` starts them"),
        "{}",
        pty.drawn()
    );

    let exported = scratch.exported(&folder);
    assert!(exported.contains("npm run dev"), "{exported}");
    assert!(!exported.contains("npm run build"), "{exported}");
    assert!(exported.contains("autostart: true"), "{exported}");
}
