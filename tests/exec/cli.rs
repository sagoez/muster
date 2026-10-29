//! The CLI end to end: the built `muster` binary run as a user would, over
//! isolated XDG roots with desktop notifications off, its projects added to
//! the database through the CLI itself, starting its own service on demand,
//! replacing an older service, refusing a newer one, and reporting what
//! happened to processes.

use std::{
    fs::{self, Permissions},
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    os::unix::{
        fs::{PermissionsExt, symlink},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    process::{self, Child, Command, Output, Stdio},
    sync::{Arc, Mutex, PoisonError},
    thread::{self, JoinHandle},
    time::{Duration, SystemTime},
};

use crossbeam_channel::unbounded;
use muster::{
    adapter::{
        holder::{
            ClientRole, ExecHolderLauncher, HOLDER_SUBCOMMAND, HolderDirs, HolderLauncher,
            HolderRunner, client::greet,
        },
        open_files::OpenFilesLimit,
        private_dir::RUNTIME_DIR_ENV,
        service::{
            BatchOutcome, EVENTS_WITHOUT_SINCE, EventKind, LaunchEnvironment, Notice, NoticeLevel,
            ProcessAction, ProcessStatus, ProjectStatus, Refusal, ServiceDeps, ServiceError,
            ServiceRequest, ServiceResponse, ServiceRuntime, ServiceVersion, Singleton,
            protocol::{self, SERVICE_SUBCOMMAND},
        },
        store::SqliteStore,
    },
    domain::{
        config::ProcessChange,
        coordination::{Author, KV_KEY_MAX_CHARS, NOTE_KEY_MAX_CHARS, TODO_DEPS_MAX},
        port::StackStore,
        process::ProcessState,
        project::Project,
        pty::PtySize,
        value::{COMMAND_MAX_CHARS, DESCRIPTION_MAX_CHARS, ProcessName, ProjectName, ProjectRoot},
    },
};
use rusqlite::Connection;
use serde_json::Value;

use crate::common::{
    BINARY, HeldLauncher, McpSession, Pty, QuietPorts, Scratch, WAIT, alive, entry,
    processes_under, result_text, stack, wait_until,
};

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
/// What a service that lost the start race writes to its log.
const LOST_RACE_LINE: &str = "another muster service already runs";
/// Why the scripted service leaves `api` alone on `up`.
const REFUSAL_REASON: &str = "cannot start: an earlier run may still be running as pid 4242";
/// The variable a started process reads from the command that started it.
const CALLER_VARIABLE: &str = "CLI_EXEC_CALLER_MARK";
/// The schema version a newer muster would have written.
const FUTURE_SCHEMA: u32 = 999;

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

/// A runner that takes a second to stop, marking that its stop began, and a
/// neighbour that first crashes once it sees that mark and then crash-loops,
/// so its first crash always falls inside the stop however slow the machine.
fn crash_loop() -> String {
    stack(&[
        entry(
            "api",
            "trap 'touch stopping; sleep 1; exit 0' TERM; while :; do sleep 0.1; done",
            true,
        ),
        entry(
            "flaky",
            "while [ ! -e stopping ]; do sleep 0.05; done; exit 1",
            true,
        )
        .with_restart("always"),
    ])
}

/// A process that ignores the stop signal for its one-second grace.
fn slow_stop() -> String {
    stack(&[entry("slow", "trap '' TERM; while :; do sleep 0.1; done", true).with_grace("1s")])
}

/// A process that writes a line every tenth of a second.
fn ticking() -> String {
    stack(&[entry(
        "ticker",
        "while :; do echo tick; sleep 0.1; done",
        true,
    )])
}

/// What the CLI suite does over a [`Scratch`].
impl Scratch {
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
    let started = scratch.socket();
    let socket = started.clone();
    let version: ServiceVersion = version.parse().expect("a test version is semantic");
    let service = thread::spawn(move || {
        let singleton = Singleton::acquire(&state_dir)?.expect("no other service runs");
        let exec = ExecHolderLauncher::builder()
            .binary(PathBuf::from(BINARY))
            .open_files(OpenFilesLimit::current())
            .build();
        let launcher: Box<dyn HolderLauncher> = if held {
            Box::new(HeldLauncher::new(exec))
        } else {
            Box::new(exec)
        };
        let store = SqliteStore::open(&database).expect("the scratch database opens");
        let recorder_store = SqliteStore::open(&database).expect("the scratch database opens");
        let deps = ServiceDeps::builder()
            .version(version)
            .binary(PathBuf::from(IN_PROCESS_BINARY))
            .socket(socket)
            .holders(Arc::new(HolderRunner::new(
                launcher,
                HolderDirs::under(&state_dir),
            )))
            .ports(Box::new(QuietPorts))
            .metrics(None)
            .stacks(Box::new(store.clone()))
            .settings(Box::new(store.clone()))
            .runs(Box::new(store.clone()))
            .raised(Box::new(store.clone()))
            .run_writes(Box::new(recorder_store.clone()))
            .raised_writes(Box::new(recorder_store))
            .edits(Box::new(store))
            .listening(None)
            .build();
        ServiceRuntime::run(singleton, deps)
    });
    wait_until("the in-process service to accept", || {
        UnixStream::connect(&started).is_ok()
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

    let services = processes_under(SERVICE_SUBCOMMAND, scratch.root());
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
    scratch.keep_child(pid);

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
    assert_eq!(processes_under(SERVICE_SUBCOMMAND, scratch.root()).len(), 1);
    assert!(alive(pid), "the process outlived the replacement");
    wait_until("the new service to adopt api", || {
        let row = scratch.row(&project, "api");
        row["state"] == "running" && row["pid"] == pid
    });
    assert_eq!(processes_under(HOLDER_SUBCOMMAND, scratch.root()).len(), 1);
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
    scratch.keep_child(pid.unwrap() as u32);
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
        !scratch.socket().exists() || replacing.try_wait().unwrap().is_some()
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
    assert_eq!(processes_under(SERVICE_SUBCOMMAND, scratch.root()).len(), 1);
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
    assert_eq!(
        slice["project"],
        serde_json::json!({"name": "shop", "root": project.to_str().unwrap()}),
        "{slice}"
    );
    let (_, unfiltered) = scratch.json_on(&project, &["logs", "dev server"]);
    assert_eq!(unfiltered["project"], slice["project"], "{unfiltered}");
    assert_eq!(unfiltered["process"], slice["process"], "{unfiltered}");
    let (_, read_on) = scratch.json_on(&project, &[
        "logs",
        "dev server",
        "--since",
        unfiltered["cursor"].as_str().unwrap(),
    ]);
    assert_eq!(
        read_on["lines_returned"], 0,
        "the plain read's cursor is its end: {read_on}"
    );
    let (_, screen) = scratch.json_on(&project, &["logs", "dev server", "--screen"]);
    assert_eq!(screen["process"], "dev server", "{screen}");
    let screen_text = screen["text"].as_str().unwrap();
    assert!(screen_text.contains("dev-ready"), "{screen}");
    assert_eq!(
        screen_text,
        screen_text.trim_end(),
        "no blank rows below the text: {screen}"
    );
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
        processes_under(SERVICE_SUBCOMMAND, scratch.root()).is_empty(),
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
        .current_dir(&project)
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

    let run_here = |args: &[&str]| {
        scratch
            .command(&[&["run", "--"], args].concat())
            .current_dir(&project)
            .output()
            .unwrap()
    };
    let first = run_here(&["sleep", "600"]);
    assert!(first.status.success(), "{}", shown(&first));
    let before = scratch.exported(&project);
    let second = run_here(&["sleep", "601"]);
    assert_eq!(
        second.status.code(),
        Some(ACTION_FAILED),
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
        processes_under(SERVICE_SUBCOMMAND, scratch.root()).is_empty(),
        "add starts nothing"
    );
    let taken = scratch.muster(&["-p", dir, "add", "web", "--", "true"]);
    assert_eq!(
        taken.status.code(),
        Some(ACTION_FAILED),
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
    assert!(human.status.success(), "{}", shown(&human));
    assert!(
        !stdout(&human).contains("muster restart"),
        "a description needs no restart: {}",
        shown(&human)
    );
    let human = scratch.on(&folder, &["set", "web", "--env", "MODE=dev"]);
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
        serde_json::json!({ "log": "ready in \\d+ms", "regex": true, "timeout": 90.0 })
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
    assert_eq!(cleared["process"]["ready"], Value::Null);

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
    assert_eq!(typo.status.code(), Some(INVALID_INPUT), "{}", shown(&typo));
    assert!(
        stderr(&typo).contains("default-stop-grace"),
        "{}",
        shown(&typo)
    );
}

/// `muster notices` reads an agent's stored edit without starting the
/// service and says none runs; once one runs, the edit is also a `notice`
/// event, as MCP `stack_events` reads them, and the note is gone.
#[test]
fn notices_read_stored_edits_without_starting_the_service() {
    let scratch = Scratch::new();
    let project = scratch.project("shop", &single());
    let store = SqliteStore::open(&scratch.database()).unwrap();
    store
        .change_process(
            &root(&project),
            &ProcessName::try_new("api").unwrap(),
            &ProcessChange::builder().autostart(Some(false)).build(),
            &Author::agent("claude"),
        )
        .unwrap();
    let edit = "agent claude changed autostart of 'api'";

    let (idle, notices) = scratch.json_on(&project, &["notices"]);

    assert!(idle.status.success(), "{}", shown(&idle));
    assert!(
        notices
            .as_array()
            .unwrap()
            .iter()
            .any(|notice| notice["message"] == edit),
        "{notices}"
    );
    assert!(
        stderr(&idle).contains("the service is not running"),
        "{}",
        shown(&idle)
    );
    assert!(processes_under(SERVICE_SUBCOMMAND, &scratch.path()).is_empty());
    assert!(!scratch.socket().exists());

    let ps = scratch.on(&project, &["ps"]);
    assert!(ps.status.success(), "{}", shown(&ps));
    let ServiceResponse::Events(page) = scratch.send(&ServiceRequest::Events {
        project: root(&project),
        since: None,
        kinds: vec![EventKind::Notice],
        wait: Duration::ZERO,
    }) else {
        panic!("events answer with a page");
    };
    assert!(
        page.events().iter().any(|event| event.detail() == edit
            && event.process().as_ref().map(AsRef::as_ref) == Some("api")),
        "{page:?}"
    );
    let live = scratch.on(&project, &["notices"]);
    assert!(live.status.success(), "{}", shown(&live));
    assert!(
        !stderr(&live).contains("the service is not running"),
        "{}",
        shown(&live)
    );
}

/// An export refuses a process working outside the project, naming it and
/// how to move it, and writes nothing; once it works in a folder under the
/// project, the exported file imports elsewhere with that folder.
#[test]
fn an_export_writes_only_working_dirs_an_import_accepts() {
    let scratch = Scratch::new();
    let shop = scratch.project("shop", &single());
    let outside = scratch.path().join("tools");
    fs::create_dir_all(&outside).unwrap();
    fs::create_dir_all(shop.join("frontend")).unwrap();
    let file = scratch.path().join("stack.yml");
    let set = scratch.on(&shop, &["set", "api", "--cwd", outside.to_str().unwrap()]);
    assert!(set.status.success(), "{}", shown(&set));

    let refused = scratch.on(&shop, &["export", file.to_str().unwrap()]);

    assert_eq!(
        refused.status.code(),
        Some(INVALID_INPUT),
        "{}",
        shown(&refused)
    );
    assert!(
        stderr(&refused).contains("`muster set 'api' --cwd <relative>`"),
        "{}",
        shown(&refused)
    );
    assert!(!file.exists(), "a refused export writes nothing");

    let set = scratch.on(&shop, &[
        "set",
        "api",
        "--cwd",
        shop.join("frontend").to_str().unwrap(),
    ]);
    assert!(set.status.success(), "{}", shown(&set));
    let export = scratch.on(&shop, &["export", file.to_str().unwrap()]);
    assert!(export.status.success(), "{}", shown(&export));
    let copy = scratch.path().join("copy");
    fs::create_dir_all(&copy).unwrap();
    let import = scratch.on(&copy, &["import", file.to_str().unwrap()]);
    assert!(import.status.success(), "{}", shown(&import));
    assert!(
        scratch.exported(&copy).contains("working_dir: frontend"),
        "{}",
        scratch.exported(&copy)
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
        Some(ACTION_FAILED),
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

/// `import --replace --dry-run` says a running process it would remove
/// would be stopped, and one not running would not, as the real run
/// reports; it stops nothing.
#[test]
fn a_replace_dry_run_says_which_removed_processes_it_would_stop() {
    let scratch = Scratch::new();
    let project = scratch.project("shop", &shop());
    let up = scratch.on(&project, &["up"]);
    assert!(up.status.success(), "{}", shown(&up));
    let smaller = scratch.path().join("smaller.yml");
    fs::write(&smaller, stack(&[entry("café", "sleep 600", true)])).unwrap();

    let (dry, planned) = scratch.json_on(&project, &[
        "import",
        smaller.to_str().unwrap(),
        "--replace",
        "--dry-run",
    ]);

    assert!(dry.status.success(), "{}", shown(&dry));
    let stopped: Vec<(&str, bool)> = planned["removed"]
        .as_array()
        .unwrap()
        .iter()
        .map(|removed| {
            (
                removed["name"].as_str().unwrap(),
                removed["stopped"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(stopped, [("dev server", true), ("api", false)], "{planned}");
    assert_eq!(scratch.row(&project, "dev server")["state"], "running");
    let text = scratch.on(&project, &[
        "import",
        smaller.to_str().unwrap(),
        "--replace",
        "--dry-run",
    ]);
    assert!(
        stdout(&text).contains("- dev server (would stop)"),
        "{}",
        shown(&text)
    );
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

    fs::write(folder.join("Procfile"), "worker: bin/worker\ndev: other\n").unwrap();
    let (more, added) = scratch.json(&["-p", dir, "init", "--yes"]);
    assert!(more.status.success(), "{}", shown(&more));
    assert_eq!(
        added["added"],
        serde_json::json!([]),
        "a name the stack has is not re-added under a source prefix"
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

/// Every client of a state directory reaches its running service, whatever
/// runtime directory the client has, or with none: the service publishes the
/// socket it bound there, so no second service is started.
#[test]
fn a_client_with_another_runtime_dir_reaches_the_running_service() {
    let scratch = Scratch::new();
    let project = scratch.project("shop", &single());
    scratch.on(&project, &["up"]);
    let services = processes_under(SERVICE_SUBCOMMAND, scratch.root());
    let elsewhere = scratch.root().join("elsewhere");

    for runtime in [Some(elsewhere.as_path()), None] {
        let mut command = scratch.command(&["-p", project.to_str().unwrap(), "ps", "--json"]);
        match runtime {
            Some(dir) => command.env(RUNTIME_DIR_ENV, dir),
            None => command.env_remove(RUNTIME_DIR_ENV),
        };
        let output = command.output().unwrap();

        assert!(output.status.success(), "{runtime:?}: {}", shown(&output));
        assert_eq!(parsed(&output)[0]["processes"][0]["state"], "running");
    }
    assert_eq!(
        processes_under(SERVICE_SUBCOMMAND, scratch.root()),
        services
    );
    assert!(
        !elsewhere.exists(),
        "the client made no socket dir of its own"
    );
}

/// A crash and its last exit outlive the service that saw them: the next
/// service lists the process crashed with how it ended, and a process
/// stopped on request is not listed crashed.
#[test]
fn a_crash_survives_a_service_restart() {
    let scratch = Scratch::new();
    let project = scratch.project(
        "broken",
        &stack(&[
            entry("web", "sleep 600", true),
            entry("broken", "echo oops; exit 3", true),
        ]),
    );
    scratch.on(&project, &["up"]);
    scratch.on(&project, &["stop", "web"]);
    let crashed = scratch.row(&project, "broken");
    assert_eq!(crashed["state"], "crashed", "{crashed}");

    let (stop, _) = scratch.json(&["service", "stop"]);
    assert!(stop.status.success(), "{}", shown(&stop));
    let restored = scratch.row(&project, "broken");
    assert_eq!(restored["state"], "crashed", "{restored}");
    assert_eq!(restored["last_exit"], crashed["last_exit"], "{restored}");
    assert_ne!(scratch.row(&project, "web")["state"], "crashed");
}

/// A start whose readiness port another program holds is refused with the
/// reason naming that program, found by the launch rather than the loop.
#[test]
fn a_start_on_a_held_port_says_why() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port().to_string();
    let scratch = Scratch::new();
    let project = scratch.project("shop", &stack(&[entry("web", "sleep 600", false)]));
    let set = scratch.on(&project, &[
        "set",
        "web",
        "--ready-port",
        &port,
        "--ready-timeout",
        "30s",
    ]);
    assert!(set.status.success(), "{}", shown(&set));

    let (started, report) = scratch.json_on(&project, &["start", "web"]);

    assert_eq!(started.status.code(), Some(ACTION_FAILED), "{report}");
    let reason = report["processes"][0]["reason"]
        .as_str()
        .unwrap_or_default();
    assert!(
        reason.starts_with(&format!(
            "did not start: port {port} is held by pid {}",
            process::id()
        )),
        "{report}"
    );
    assert_eq!(scratch.row(&project, "web")["state"], "blocked");
    drop(listener);
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
    Connection::open(scratch.database())
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
    scratch.prepare_socket_dirs();
    let listener = UnixListener::bind(scratch.socket()).unwrap();
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
                    version: ServiceVersion::current(),
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
    scratch.prepare_socket_dirs();
    let listener = UnixListener::bind(scratch.socket()).unwrap();
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
                    version: ServiceVersion::current(),
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
    scratch.prepare_socket_dirs();
    let listener = UnixListener::bind(scratch.socket()).unwrap();
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
        processes_under(SERVICE_SUBCOMMAND, scratch.root()).is_empty(),
        "no service was started"
    );

    let usage = scratch.muster(&["start", "--bogus", "--json"]);
    assert_eq!(usage.status.code(), Some(USAGE));
    assert_eq!(parsed(&usage)["error"]["kind"], "usage");

    for (refused, kind) in [
        (&["wait", "api", "--timeout", "0s"][..], "invalid-input"),
        (
            &["wait", "api", "--http", "http://example.com/"][..],
            "invalid-input",
        ),
        (&["config", "get", "detach_key"][..], "invalid-input"),
        (
            &["logs", "api", "--grep", "(", "--regex"][..],
            "invalid-input",
        ),
        (&["-p", "nosuch", "ps"][..], "unknown-project"),
    ] {
        let plain = scratch.muster(refused);
        assert_eq!(
            plain.status.code(),
            Some(INVALID_INPUT),
            "{refused:?} is an invalid value: {}",
            shown(&plain)
        );
        let mut json = refused.to_vec();
        json.push("--json");
        let answered = scratch.muster(&json);
        assert_eq!(answered.status.code(), Some(INVALID_INPUT), "{refused:?}");
        assert_eq!(parsed(&answered)["error"]["kind"], kind, "{refused:?}");
    }

    let unknown = scratch
        .command(&["ps", "--json"])
        .env("MUSTER_PROJECT", "nosuch")
        .output()
        .unwrap();
    assert_eq!(
        unknown.status.code(),
        Some(INVALID_INPUT),
        "{}",
        shown(&unknown)
    );
    let message = parsed(&unknown)["error"]["message"].to_string();
    assert!(
        message.contains("nosuch") && !message.contains("not a directory"),
        "an unknown MUSTER_PROJECT name is an unknown project: {message}"
    );
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

/// A hand-corrupted process row breaks only its own project: `projects`
/// lists the others and marks it, `doctor` names it, and `projects rm`
/// still forgets it.
#[test]
fn a_corrupt_process_row_breaks_only_its_project() {
    let scratch = Scratch::new();
    scratch.project("shop", &shop());
    scratch.project("blog", &single());
    Connection::open(scratch.database())
        .unwrap()
        .execute(
            "UPDATE processes SET stop_grace_ms = 7200000 WHERE project_id = \
             (SELECT id FROM projects WHERE name = 'blog')",
            [],
        )
        .unwrap();

    let (listed, projects) = scratch.json(&["projects"]);
    assert!(listed.status.success(), "{}", shown(&listed));
    let named = |name: &str| {
        projects
            .as_array()
            .unwrap()
            .iter()
            .find(|project| project["name"] == name)
            .cloned()
            .unwrap_or_else(|| panic!("no project {name}: {projects}"))
    };
    assert_eq!(named("shop")["processes"], 3);
    assert_eq!(named("blog")["processes"], Value::Null);
    assert!(
        named("blog")["invalid"]
            .as_str()
            .unwrap()
            .contains("stop grace"),
        "{projects}"
    );

    let doctor = scratch.muster(&["doctor"]);
    assert_eq!(
        doctor.status.code(),
        Some(ACTION_FAILED),
        "{}",
        shown(&doctor)
    );
    assert!(
        stdout(&doctor).contains("blog (stored stop grace is invalid"),
        "{}",
        shown(&doctor)
    );

    let (rm, removal) = scratch.json(&["projects", "rm", "blog", "--yes"]);
    assert!(rm.status.success(), "{}", shown(&rm));
    assert_eq!(removal["removed"], true);
    assert_eq!(removal["stopped"], Value::Null);
    let (_, projects) = scratch.json(&["projects"]);
    assert_eq!(projects.as_array().unwrap().len(), 1, "{projects}");
    assert_eq!(projects[0]["name"], "shop");
    let doctor = scratch.muster(&["doctor"]);
    assert!(
        stdout(&doctor).contains("every stored process reads"),
        "{}",
        shown(&doctor)
    );
}

/// A stored working directory that climbs out of the project, which only a
/// hand edit of the database can make, is refused naming the process, and
/// the process never starts.
#[test]
fn a_stored_working_dir_climbing_out_of_the_project_never_starts() {
    let scratch = Scratch::new();
    let project = scratch.project(
        "shop",
        &stack(&[entry("escape", "pwd > ran; sleep 600", false)]),
    );
    Connection::open(scratch.database())
        .unwrap()
        .execute(
            "UPDATE processes SET working_dir = '../../../../etc' WHERE name = 'escape'",
            [],
        )
        .unwrap();

    let start = scratch.on(&project, &["start", "escape"]);

    assert!(!start.status.success(), "{}", shown(&start));
    assert!(
        stderr(&start).contains("working dir of 'escape'"),
        "{}",
        shown(&start)
    );
    assert!(
        processes_under(HOLDER_SUBCOMMAND, scratch.root()).is_empty(),
        "nothing was launched"
    );
}

/// A project row edited by hand to a name muster refuses breaks only that
/// project: every other project keeps working, `projects` lists it by its
/// folder, a command on it names that folder, `doctor` names it, and
/// `projects rm <folder>` forgets it.
#[test]
fn a_corrupt_project_row_breaks_only_that_project() {
    let scratch = Scratch::new();
    let shop = scratch.project("shop", &single());
    let blog = scratch.project("blog", &single());
    Connection::open(scratch.database())
        .unwrap()
        .execute(
            "UPDATE projects SET name = 'bad\u{1b}]2;pwn\u{7}' WHERE name = 'blog'",
            [],
        )
        .unwrap();
    let blog_root = blog.canonicalize().unwrap().display().to_string();

    let (listed, projects) = scratch.json(&["projects"]);
    assert!(listed.status.success(), "{}", shown(&listed));
    let rows = projects.as_array().unwrap();
    assert_eq!(rows.len(), 2, "{projects}");
    let broken = rows
        .iter()
        .find(|row| row["root"] == blog_root.as_str())
        .unwrap_or_else(|| panic!("no row for {blog_root}: {projects}"));
    assert_eq!(broken["name"], Value::Null, "{projects}");
    assert!(broken["invalid"].is_string(), "{projects}");

    let (healthy, _) = scratch.json_on(&shop, &["ps"]);
    assert!(healthy.status.success(), "{}", shown(&healthy));
    let named = scratch.on(&blog, &["ps"]);
    assert!(!named.status.success(), "{}", shown(&named));
    assert!(stderr(&named).contains(&blog_root), "{}", shown(&named));
    assert!(!stderr(&named).contains('\u{1b}'), "{}", shown(&named));
    let doctor = scratch.muster(&["doctor"]);
    assert!(stdout(&doctor).contains(&blog_root), "{}", shown(&doctor));

    let (rm, removal) = scratch.json(&["projects", "rm", &blog_root, "--yes"]);
    assert!(rm.status.success(), "{}", shown(&rm));
    assert_eq!(removal["removed"], true, "{removal}");
    let (_, projects) = scratch.json(&["projects"]);
    assert_eq!(projects.as_array().unwrap().len(), 1, "{projects}");
    assert_eq!(projects[0]["name"], "shop");
}

/// A project row whose stored root is not absolute is listed under that text
/// and `projects rm` with the same text forgets it, leaving the others.
#[test]
fn a_project_row_with_a_relative_root_is_removed_by_its_listed_text() {
    let scratch = Scratch::new();
    scratch.project("shop", &single());
    scratch.project("blog", &single());
    Connection::open(scratch.database())
        .unwrap()
        .execute(
            "UPDATE projects SET root = 'relative/blog' WHERE name = 'blog'",
            [],
        )
        .unwrap();

    let (_, projects) = scratch.json(&["projects"]);
    let listed = projects
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["name"].is_null())
        .unwrap_or_else(|| panic!("no unreadable row: {projects}"))["root"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(listed, "relative/blog", "{projects}");

    let (rm, removal) = scratch.json(&["projects", "rm", &listed, "--yes"]);
    assert!(rm.status.success(), "{}", shown(&rm));
    assert_eq!(removal["removed"], true, "{removal}");
    assert_eq!(removal["root"], "relative/blog", "{removal}");
    let (_, projects) = scratch.json(&["projects"]);
    assert_eq!(projects.as_array().unwrap().len(), 1, "{projects}");
    assert_eq!(projects[0]["name"], "shop");
}

/// A setting or field past its bound is refused as invalid input, never as a
/// database error, and changes nothing; `config set` answers with the value
/// the database holds.
#[test]
fn values_past_their_bounds_are_invalid_input() {
    let scratch = Scratch::new();
    for (key, value) in [
        ("scrollback", "100001"),
        ("scrollback", "18446744073709551616"),
        ("log-max-size", "1KiB"),
        ("log-max-size", "2GiB"),
        ("default-stop-grace", "2h"),
        ("default-stop-grace", "300000000y"),
    ] {
        let refused = scratch.muster(&["config", "set", key, value]);
        assert_eq!(
            refused.status.code(),
            Some(INVALID_INPUT),
            "{key} {value}: {}",
            shown(&refused)
        );
        assert!(
            !stderr(&refused).contains("database"),
            "{}",
            shown(&refused)
        );
    }
    let (_, scrollback) = scratch.json(&["config", "get", "scrollback"]);
    assert_eq!(scrollback["value"], "1000");

    let (set, stored) = scratch.json(&["config", "set", "default-stop-grace", "1500us"]);
    assert!(set.status.success(), "{}", shown(&set));
    assert_eq!(stored["value"], "2ms");

    let dir = scratch.path().join("shop");
    fs::create_dir_all(&dir).unwrap();
    let project = dir.to_str().unwrap();
    let endless = scratch.muster(&[
        "-p",
        project,
        "add",
        "web",
        "--ready-port",
        "3000",
        "--ready-timeout",
        "300000000y",
        "--",
        "sleep 600",
    ]);
    assert_eq!(
        endless.status.code(),
        Some(INVALID_INPUT),
        "{}",
        shown(&endless)
    );
    assert!(
        !stderr(&endless).contains("database"),
        "{}",
        shown(&endless)
    );
    let long_grace = scratch.muster(&[
        "-p",
        project,
        "add",
        "web",
        "--stop-grace",
        "2h",
        "--",
        "sleep 600",
    ]);
    assert!(!long_grace.status.success(), "{}", shown(&long_grace));
    let (_, projects) = scratch.json(&["projects"]);
    assert_eq!(projects, serde_json::json!([]));
}

/// Completing `--project` reads an existing database and never creates one.
#[test]
fn completing_a_project_never_creates_the_database() {
    let complete = |scratch: &Scratch| {
        scratch
            .command(&["--", "muster", "--project", ""])
            .env("COMPLETE", "fish")
            .output()
            .unwrap()
    };
    let bare = Scratch::empty();

    let empty = complete(&bare);
    assert!(empty.status.success(), "{}", shown(&empty));
    assert!(!bare.state_root().exists());

    let scratch = Scratch::new();
    scratch.project("shop", &single());
    let listed = complete(&scratch);
    assert!(listed.status.success(), "{}", shown(&listed));
    assert!(stdout(&listed).contains("shop"), "{}", shown(&listed));
}

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

/// With no service and no other muster running, an agent's MCP server starts
/// the service and drives the project's processes by name: it lists them,
/// starts, types into, reads, restarts and stops them, and an unknown name
/// is refused with the valid ones.
#[test]
fn mcp_tools_work_with_the_service_autostarted() {
    let mut scratch = Scratch::new();
    let project = scratch.project("shop", &mcp_project());
    assert!(processes_under(SERVICE_SUBCOMMAND, scratch.root()).is_empty());
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
        !processes_under(SERVICE_SUBCOMMAND, scratch.root()).is_empty(),
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
    let server = mcp.pid();
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
        serde_json::json!({ "name": "api", "ready": { "log": "ready on", "timeout": 30 }, "stop_grace": 2.5 }),
    );
    assert_eq!(
        set["structuredContent"]["process"]["ready"],
        serde_json::json!({ "log": "ready on", "timeout": 30.0 }),
        "{set}"
    );
    assert_eq!(
        set["structuredContent"]["process"]["stop"]["grace"], 2.5,
        "{set}"
    );
    for humantime in [
        serde_json::json!({ "name": "api", "ready": { "log": "ready on", "timeout": "30s" } }),
        serde_json::json!({ "name": "api", "stop_grace": "5s" }),
    ] {
        let refused = mcp.call("process_set", humantime).to_string();
        assert!(refused.contains("number of seconds"), "{refused}");
    }
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

/// A start sees a child that dies moments after it runs: `start` and
/// `start --wait` fail, and MCP `process_start` with `wait: true` flags the
/// crash as an error, with or without a restart policy.
#[test]
fn a_waited_start_reports_a_child_that_dies_at_once() {
    let mut scratch = Scratch::new();
    let project = scratch.project(
        "shop",
        &stack(&[
            entry("bad", "sleep 0.2; exit 3", false),
            entry("ronfail", "sleep 0.2; exit 3", false).with_restart("on-failure"),
        ]),
    );

    let (started, report) = scratch.json_on(&project, &["start", "bad", "--wait"]);
    assert_eq!(started.status.code(), Some(ACTION_FAILED), "{report}");
    assert_eq!(report["status"], "crashed", "{report}");
    let (unwaited, report) = scratch.json_on(&project, &["start", "bad"]);
    assert_eq!(unwaited.status.code(), Some(ACTION_FAILED), "{report}");
    assert_eq!(report["processes"][0]["outcome"], "crashed", "{report}");

    let mut mcp = McpSession::start(&mut scratch, &project);
    let waited = mcp.result(
        "process_start",
        serde_json::json!({ "name": "ronfail", "wait": true }),
    );
    assert_eq!(waited["isError"], true, "{waited}");
    assert_eq!(
        waited["structuredContent"]["wait"]["status"], "crashed",
        "{waited}"
    );
    mcp.result("process_stop", serde_json::json!({ "name": "ronfail" }));
}

/// A waited `process_start` of a process already running says so, as the
/// unwaited start does; one that starts it does not.
#[test]
fn a_waited_start_of_a_running_process_says_it_was_already_running() {
    let mut scratch = Scratch::new();
    let project = scratch.project("shop", &stack(&[entry("api", "echo up; sleep 600", false)]));
    let mut mcp = McpSession::start(&mut scratch, &project);
    let start = |mcp: &mut McpSession| {
        mcp.result(
            "process_start",
            serde_json::json!({ "name": "api", "wait": true }),
        )["structuredContent"]
            .clone()
    };

    let first = start(&mut mcp);
    assert_eq!(first["wait"]["status"], "matched", "{first}");
    assert!(first.get("outcome").is_none(), "{first}");
    let again = start(&mut mcp);
    assert_eq!(again["outcome"], "already-running", "{again}");
    assert_eq!(again["wait"]["status"], "matched", "{again}");
    let unwaited = mcp.result("process_start", serde_json::json!({ "name": "api" }));
    assert_eq!(
        unwaited["structuredContent"]["outcome"], "already-running",
        "{unwaited}"
    );
    mcp.result("process_stop", serde_json::json!({ "name": "api" }));
}

/// Reading events filtered to other kinds never marks a crash heard: the
/// next stack call still alerts it.
#[test]
fn a_filtered_event_read_leaves_a_crash_alert_for_the_next_call() {
    let mut scratch = Scratch::new();
    let project = scratch.project(
        "shop",
        &stack(&[entry("flaky", "echo boom; exit 3", false)]),
    );
    let mut mcp = McpSession::start(&mut scratch, &project);
    mcp.result("stack_status", serde_json::json!({}));

    scratch.on(&project, &["start", "flaky"]);
    wait_until("the crash", || {
        scratch.row(&project, "flaky")["state"] == "crashed"
    });
    let events = mcp.result(
        "stack_events",
        serde_json::json!({ "kinds": ["started"] }),
    )["structuredContent"]
        .clone();
    assert!(events.get("alerts").is_none(), "{events}");
    let kinds: Vec<&str> = events["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["started"], "{events}");

    let status = mcp.result("stack_status", serde_json::json!({}))["structuredContent"].clone();
    assert!(
        status["alerts"]
            .as_array()
            .is_some_and(|alerts| alerts.iter().any(|alert| alert["kind"] == "crashed")),
        "{status}"
    );
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
        processes_under(SERVICE_SUBCOMMAND, scratch.root()).is_empty(),
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
    scratch.keep_child(pid as u32);
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

/// A request the stored state refuses exits 1 with the kind `conflict`,
/// changing nothing: a name the project already has, an import whose names
/// the project keeps, and registering a folder that holds a project. A
/// malformed name still exits 4 as invalid input.
#[test]
fn a_refusal_over_state_exits_1_as_a_conflict() {
    let scratch = Scratch::new();
    let project = scratch.project("outer/shop", &single());
    let shop = project.to_str().unwrap();
    let outer = scratch.path().join("outer");
    let before = scratch.exported(&project);

    let answered = |args: &[&str]| {
        let output = scratch.muster(args);
        let failure = parsed(&output);
        (output, failure)
    };

    let refusals = [
        (
            "a taken name",
            answered(&["-p", shop, "add", "--json", "api", "--", "true"]),
        ),
        (
            "an import conflict",
            answered(&["-p", shop, "import", "--json"]),
        ),
        (
            "a folder holding a project",
            answered(&[
                "-p",
                outer.to_str().unwrap(),
                "add",
                "--json",
                "web",
                "--",
                "true",
            ]),
        ),
    ];

    for (what, (output, failure)) in refusals {
        assert_eq!(
            output.status.code(),
            Some(ACTION_FAILED),
            "{what}: {}",
            shown(&output)
        );
        assert_eq!(failure["error"]["kind"], "conflict", "{what}: {failure}");
    }
    assert_eq!(scratch.exported(&project), before, "nothing changed");
    let (projects, listed) = scratch.json(&["projects"]);
    assert!(projects.status.success(), "{}", shown(&projects));
    assert_eq!(listed.as_array().map(Vec::len), Some(1), "{listed}");
    let (malformed, failure) =
        answered(&["-p", shop, "add", "--json", "line\nbreak", "--", "true"]);
    assert_eq!(
        malformed.status.code(),
        Some(INVALID_INPUT),
        "{}",
        shown(&malformed)
    );
    assert_eq!(failure["error"]["kind"], "invalid-input", "{failure}");
}

/// Changing a todo another agent holds is a refusal over state: it exits 1
/// with the kind `conflict`, not as an invalid value, and `--force` takes it.
#[test]
fn a_todo_held_by_an_agent_is_refused_as_a_conflict() {
    let mut scratch = Scratch::new();
    let project = scratch.project("shop", &single());
    let mut mcp = McpSession::start(&mut scratch, &project);
    let added = mcp.result("todo_add", serde_json::json!({ "title": "ship" }));
    let id = added["structuredContent"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let claimed = mcp.result("todo_claim", serde_json::json!({ "id": id }));
    assert_eq!(claimed["isError"], false, "{claimed}");

    let held = scratch.on(&project, &["todo", "status", &id, "done", "--json"]);
    assert_eq!(held.status.code(), Some(ACTION_FAILED), "{}", shown(&held));
    assert_eq!(
        parsed(&held)["error"]["kind"],
        "conflict",
        "{}",
        shown(&held)
    );
    let plain = scratch.on(&project, &["todo", "status", &id, "done"]);
    assert_eq!(
        plain.status.code(),
        Some(ACTION_FAILED),
        "{}",
        shown(&plain)
    );

    let forced = scratch.on(&project, &["todo", "status", &id, "done", "--force"]);
    assert!(forced.status.success(), "{}", shown(&forced));
}

/// Every field an agent writes has a named maximum: a key, command,
/// description or dependency list past it is refused naming the limit, over
/// MCP and the CLI alike, and after maximal writes `stack_status` and
/// `stack_events` stay bounded.
#[test]
fn agent_written_fields_are_bounded() {
    let mut scratch = Scratch::new();
    let project = scratch.project("shop", &single());
    let mut mcp = McpSession::start(&mut scratch, &project);
    let long = |max: usize| "x".repeat(max + 1);

    for (tool, arguments, limit) in [
        (
            "note_write",
            serde_json::json!({ "key": long(NOTE_KEY_MAX_CHARS), "body": "b" }),
            NOTE_KEY_MAX_CHARS,
        ),
        (
            "kv_write",
            serde_json::json!({ "key": long(KV_KEY_MAX_CHARS), "value": "v" }),
            KV_KEY_MAX_CHARS,
        ),
        (
            "todo_add",
            serde_json::json!({ "title": "t", "deps": vec!["a"; TODO_DEPS_MAX + 1] }),
            TODO_DEPS_MAX,
        ),
        (
            "process_add",
            serde_json::json!({ "name": "big", "command": long(COMMAND_MAX_CHARS) }),
            COMMAND_MAX_CHARS,
        ),
        (
            "process_add",
            serde_json::json!({
                "name": "big",
                "command": "true",
                "description": long(DESCRIPTION_MAX_CHARS),
            }),
            DESCRIPTION_MAX_CHARS,
        ),
    ] {
        let refused = mcp.mistake(tool, arguments);
        assert!(
            result_text(&refused).contains(&format!("at most {limit}")),
            "{tool}: {refused}"
        );
    }

    let maximal = |mark: &str| format!("true {mark}{}", "#".repeat(COMMAND_MAX_CHARS - 6));
    mcp.result(
        "process_add",
        serde_json::json!({
            "name": "big",
            "command": maximal("a"),
            "description": "d".repeat(DESCRIPTION_MAX_CHARS),
        }),
    );
    for mark in ["b", "c", "d"] {
        mcp.result(
            "process_set",
            serde_json::json!({ "name": "big", "command": maximal(mark) }),
        );
    }
    let bound = 4 * (COMMAND_MAX_CHARS + DESCRIPTION_MAX_CHARS) * EVENTS_WITHOUT_SINCE;
    for tool in ["stack_status", "stack_events"] {
        let size = mcp.result(tool, serde_json::json!({})).to_string().len();
        assert!(size < bound, "{tool} answered {size} bytes");
    }

    let cli = scratch.on(&project, &["note", "set", &long(NOTE_KEY_MAX_CHARS), "b"]);
    assert_eq!(cli.status.code(), Some(INVALID_INPUT), "{}", shown(&cli));
    assert!(
        stderr(&cli).contains(&format!("at most {NOTE_KEY_MAX_CHARS}")),
        "{}",
        shown(&cli)
    );
    let cli = scratch.on(&project, &["add", "huge", "--", &long(COMMAND_MAX_CHARS)]);
    assert_eq!(cli.status.code(), Some(INVALID_INPUT), "{}", shown(&cli));
    assert!(
        stderr(&cli).contains(&format!("at most {COMMAND_MAX_CHARS}")),
        "{}",
        shown(&cli)
    );
    for set in [
        vec!["set", "big", "--command", &long(COMMAND_MAX_CHARS)],
        vec!["set", "big", "--", &long(COMMAND_MAX_CHARS)],
    ] {
        let cli = scratch.on(&project, &set);
        assert_eq!(cli.status.code(), Some(INVALID_INPUT), "{}", shown(&cli));
        assert!(
            stderr(&cli).contains(&format!("at most {COMMAND_MAX_CHARS}")),
            "{}",
            shown(&cli)
        );
        assert!(
            stderr(&cli).len() < COMMAND_MAX_CHARS,
            "the refusal echoes the command: {}",
            shown(&cli)
        );
        let mut json = set.clone();
        json.insert(2, "--json");
        let refused = scratch.on(&project, &json);
        assert_eq!(
            refused.status.code(),
            Some(INVALID_INPUT),
            "{}",
            shown(&refused)
        );
        assert_eq!(
            parsed(&refused)["error"]["kind"],
            "invalid-input",
            "{}",
            shown(&refused)
        );
    }
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

/// The Enter key, as a person types it at a prompt.
const ENTER: &[u8] = b"\r";
/// The down-arrow key, as a person types it at a prompt.
const ARROW_DOWN: &[u8] = b"\x1b[B";
/// The default detach key, `ctrl-\`.
const DETACH: &[u8] = b"\x1c";

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
        processes_under(SERVICE_SUBCOMMAND, scratch.root()).is_empty(),
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
        &stack(&[
            entry("api", "sleep 600", true),
            entry("slow", "trap '' TERM; while :; do sleep 0.1; done", true).with_grace("1s"),
        ]),
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
    pty.wait_for("no process", 1);

    let up = scratch.on(&project, &["up"]);
    assert!(up.status.success(), "{}", shown(&up));
    let pty = Pty::open();
    let mut attach = pty.spawn(&mut scratch, &project, &["attach"]);
    pty.wait_for("Which process?", 1);
    pty.wait_for("café", 1);
    pty.wait_for("running", 1);
    let drawn = pty.drawn();
    assert!(
        !drawn.contains("○ api") && !drawn.contains("● api"),
        "{drawn}"
    );
    pty.type_keys(ARROW_DOWN);
    pty.type_keys(ENTER);
    pty.wait_for("attached to café", 1);
    pty.type_keys(DETACH);
    assert_eq!(Pty::exit_code(&mut attach), Some(0), "{}", pty.drawn());
    pty.wait_for("detached from café", 1);
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
    let attached_size = PtySize::try_new(ATTACH_ROWS, ATTACH_COLS).unwrap();

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

/// A process that prints colour, a window title, a clipboard write, a
/// device query, a window report query and a DCS, redraws a progress line,
/// then prints `done`.
const NOISY: &str = "printf '\\033[31mred\\033[0m \\033]0;pwned\\007 \\033]52;c;aGk=\\007 \\033[6n \\033[18t \\033P1$qm\\033\\\\ 10%%\\r\\033[2K50%%\\033[1A done\\n'; sleep 600";
/// The progress redraw [`NOISY`] prints, which a terminal still receives.
const PROGRESS_REDRAW: &str = "10%\r\x1b[2K50%\x1b[1A";

/// Whether `haystack` holds `needle`.
fn holds(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

/// On a terminal, `logs` and `logs -f` show a process's text, colours and
/// progress redraws but nothing that could retitle the terminal, write its
/// clipboard or query it; a pipe still receives every byte the process wrote.
#[test]
fn logs_on_a_terminal_drop_what_could_drive_it() {
    let mut scratch = Scratch::new();
    let project = scratch.project("shop", &stack(&[entry("noisy", NOISY, true)]));
    let up = scratch.on(&project, &["up"]);
    assert!(up.status.success(), "{}", shown(&up));
    wait_until("the noisy output to be logged", || {
        stdout(&scratch.on(&project, &["logs", "noisy"])).contains("done")
    });

    let piped = scratch.on(&project, &["logs", "noisy"]);
    assert!(
        holds(&piped.stdout, b"\x1b]0;pwned\x07"),
        "{}",
        shown(&piped)
    );
    assert!(holds(&piped.stdout, b"\x1b[6n"), "{}", shown(&piped));
    assert!(holds(&piped.stdout, b"\x1b[18t"), "{}", shown(&piped));
    assert!(
        holds(&piped.stdout, PROGRESS_REDRAW.as_bytes()),
        "{}",
        shown(&piped)
    );

    for args in [&["logs", "noisy"][..], &["logs", "-f", "noisy"][..]] {
        let pty = Pty::open();
        let mut logs = pty.spawn(&mut scratch, &project, args);
        pty.wait_for("done", 1);
        let _ = logs.kill();
        let _ = logs.wait();
        let raw = pty.raw();
        assert!(raw.contains("\x1b[31mred\x1b[0m"), "{args:?}: {raw:?}");
        assert!(raw.contains(PROGRESS_REDRAW), "{args:?}: {raw:?}");
        for driving in [
            "pwned", "\x1b]", "aGk=", "\x1b[6n", "\x1b[18t", "\x1bP", "1$qm",
        ] {
            assert!(
                !raw.contains(driving),
                "{args:?} let {driving:?} through: {raw:?}"
            );
        }
    }
}

/// `export -` on a terminal escapes the control and bidi characters a
/// stored field holds, so they cannot reorder or drive what is shown; a
/// pipe still receives the file exactly.
#[test]
fn export_to_a_terminal_escapes_what_a_pipe_keeps() {
    let mut scratch = Scratch::new();
    let project = scratch.project("shop", &single());
    let added = scratch.on(&project, &[
        "add",
        "tricky",
        "--description",
        "left\u{202e}right",
        "--",
        "sleep 1",
    ]);
    assert!(added.status.success(), "{}", shown(&added));

    let piped = scratch.exported(&project);
    assert!(piped.contains("left\u{202e}right"), "{piped:?}");

    let pty = Pty::open();
    let mut export = pty.spawn(&mut scratch, &project, &["export", "-"]);
    assert_eq!(Pty::exit_code(&mut export), Some(0));
    pty.wait_for("right", 1);
    let raw = pty.raw();
    assert!(raw.contains("left\\u{202e}right"), "{raw:?}");
    assert!(!raw.contains('\u{202e}'), "{raw:?}");
}

/// `import` reads only a regular file of bounded size, refuses a working
/// directory outside the project, and shows what each process will run
/// and where before anything is written.
#[test]
fn import_shows_what_it_runs_and_refuses_a_hostile_file() {
    let scratch = Scratch::new();
    let folder = scratch.path().join("shop");
    fs::create_dir_all(&folder).unwrap();
    let import = |file: &Path, extra: &[&str]| {
        let mut args = vec!["import", file.to_str().unwrap()];
        args.extend_from_slice(extra);
        scratch.on(&folder, &args)
    };

    let fifo = folder.join("fifo.yml");
    let made = Command::new("mkfifo").arg(&fifo).status().unwrap();
    assert!(made.success());
    let refused = import(&fifo, &[]);
    assert!(!refused.status.success(), "{}", shown(&refused));
    assert!(
        stderr(&refused).contains("not a regular file"),
        "{}",
        shown(&refused)
    );

    let large = folder.join("large.yml");
    fs::write(&large, "#".repeat(2 * 1024 * 1024)).unwrap();
    let refused = import(&large, &[]);
    assert!(
        stderr(&refused).contains("too large"),
        "{}",
        shown(&refused)
    );

    let entry = |dir: &str| {
        format!(
            "version: 1\nprocesses:\n  - name: web\n    command: npm run dev\n    working_dir: '{dir}'\n    autostart: true\n    restart: never\n    stop:\n      signal: terminate\n      grace_period: 5s\n"
        )
    };
    let file = folder.join("muster.yml");
    for dir in ["/etc", "../elsewhere"] {
        fs::write(&file, entry(dir)).unwrap();
        let refused = import(&file, &[]);
        assert!(
            stderr(&refused).contains(&format!(
                "processes[0] (web): working_dir {dir} must be a folder under the project root"
            )),
            "{}",
            shown(&refused)
        );
    }

    let outside = scratch.path().join("outside");
    fs::create_dir_all(folder.join("sub")).unwrap();
    fs::create_dir_all(&outside).unwrap();
    symlink(&outside, folder.join("sub/link")).unwrap();
    fs::write(&file, entry("sub/link")).unwrap();
    let refused = import(&file, &[]);
    assert_eq!(
        refused.status.code(),
        Some(INVALID_INPUT),
        "{}",
        shown(&refused)
    );
    assert!(
        stderr(&refused).contains("outside the project root"),
        "{}",
        shown(&refused)
    );

    fs::write(&file, entry("frontend")).unwrap();
    let (dry_json, planned) =
        scratch.json_on(&folder, &["import", file.to_str().unwrap(), "--dry-run"]);
    assert!(dry_json.status.success(), "{}", shown(&dry_json));
    assert_eq!(planned["registered"], false, "{planned}");
    assert_eq!(planned["would_register"], true, "{planned}");
    let dry = import(&file, &["--dry-run"]);
    assert!(dry.status.success(), "{}", shown(&dry));
    let shown_dry = stdout(&dry);
    assert!(shown_dry.contains("would register"), "{shown_dry}");
    assert!(
        shown_dry.contains("+ web  npm run dev  in frontend"),
        "{shown_dry}"
    );
    let listed = scratch.muster(&["projects", "--json"]);
    assert!(!stdout(&listed).contains("shop"), "{}", shown(&listed));

    fs::write(&file, "version: 1\nprocesses: []\n").unwrap();
    let empty = import(&file, &[]);
    assert!(empty.status.success(), "{}", shown(&empty));
    assert!(stdout(&empty).contains("registered"), "{}", shown(&empty));
    assert!(
        !stdout(&empty).contains("nothing to change"),
        "{}",
        shown(&empty)
    );
}

/// When `MUSTER_PROJECT` makes an editing command act on a project other
/// than the one the current folder is in, the command says so on stderr;
/// inside that same project it says nothing.
#[test]
fn an_overriding_muster_project_is_noted() {
    let scratch = Scratch::new();
    let shop = scratch.project("shop", &single());
    let blog = scratch.project("blog", &single());
    let in_folder = |folder: &Path, args: &[&str]| {
        scratch
            .command(args)
            .current_dir(folder)
            .env("MUSTER_PROJECT", &shop)
            .output()
            .unwrap()
    };

    let added = in_folder(&blog, &["add", "worker", "--", "sleep", "1"]);
    assert!(added.status.success(), "{}", shown(&added));
    assert!(
        stderr(&added).contains("MUSTER_PROJECT names shop")
            && stderr(&added).contains("not on blog"),
        "{}",
        shown(&added)
    );
    assert!(scratch.exported(&shop).contains("worker"));
    assert!(!scratch.exported(&blog).contains("worker"));

    let quiet = in_folder(&shop, &["set", "worker", "--description", "a worker"]);
    assert!(quiet.status.success(), "{}", shown(&quiet));
    assert!(
        !stderr(&quiet).contains("MUSTER_PROJECT"),
        "{}",
        shown(&quiet)
    );
}

/// The escape character no terminal-safe text may carry.
const ESCAPE: char = '\u{1b}';
/// Mode bits a permission check compares.
const PERMISSION_BITS: u32 = 0o777;
/// A file mode other users can read.
const SHARED_FILE_MODE: u32 = 0o644;
/// The mode of a file only its owner can read.
const PRIVATE_FILE_MODE: u32 = 0o600;
/// Characters in a state folder name that leave no room for a socket path.
const DEEP_STATE_DIR_CHARS: usize = 120;
/// Length of a state home whose holder sockets just fit on every platform,
/// but where a service socket bound beside them would not have.
const LONG_STATE_HOME_CHARS: usize = 40;

/// A folder whose name carries an OSC title sequence registers under that
/// name escaped, so neither `projects` nor shell completion sends the
/// sequence to a terminal.
#[test]
fn a_folder_named_with_terminal_sequences_registers_an_escaped_name() {
    let scratch = Scratch::new();
    let hostile = scratch.project("x\u{1b}]0;pwned\u{7}", &single());

    let (listed, projects) = scratch.json(&["projects"]);
    assert!(listed.status.success(), "{}", shown(&listed));
    let name = projects
        .as_array()
        .unwrap()
        .iter()
        .find(|project| project["root"] == hostile.to_str().unwrap())
        .map(|project| project["name"].as_str().unwrap().to_string())
        .unwrap();
    assert_eq!(name, "x\\u{1b}]0;pwned\\u{7}");

    let completed = scratch
        .command(&["--", "muster", "--project", ""])
        .env("COMPLETE", "fish")
        .output()
        .unwrap();
    assert!(completed.status.success(), "{}", shown(&completed));
    assert!(stdout(&completed).contains(&name), "{}", shown(&completed));
    assert!(
        !stdout(&completed).contains(ESCAPE),
        "{}",
        shown(&completed)
    );
}

/// `add` names why a name is refused and shows it escaped.
#[test]
fn add_explains_an_invalid_name_and_escapes_it() {
    let scratch = Scratch::new();
    let shop = scratch.project("shop", &single());

    let refused = scratch.on(&shop, &["add", "web\u{1b}[2J", "--", "true"]);

    assert_eq!(
        refused.status.code(),
        Some(INVALID_INPUT),
        "{}",
        shown(&refused)
    );
    assert!(
        stderr(&refused).contains("'web\\u{1b}[2J' is not a valid process name")
            && stderr(&refused).contains("holds a control or format character"),
        "{}",
        shown(&refused)
    );
    assert!(!stderr(&refused).contains(ESCAPE), "{}", shown(&refused));
}

/// `export --with-env` writes environment values its owner alone can read,
/// even over a file others could.
#[test]
fn an_export_with_env_values_is_private() {
    let scratch = Scratch::new();
    let shop = scratch.project("shop", &single());
    let file = scratch.path().join("secrets.yml");
    fs::write(&file, "old").unwrap();
    fs::set_permissions(&file, Permissions::from_mode(SHARED_FILE_MODE)).unwrap();

    let exported = scratch.on(&shop, &[
        "export",
        file.to_str().unwrap(),
        "--with-env",
        "--force",
    ]);

    assert!(exported.status.success(), "{}", shown(&exported));
    let mode = fs::metadata(&file).unwrap().permissions().mode() & PERMISSION_BITS;
    assert_eq!(mode, PRIVATE_FILE_MODE);
}

/// A state directory too deep for a Unix socket is refused before a service
/// is started, naming the path and what to change.
#[test]
fn a_state_dir_too_deep_for_a_socket_is_reported() {
    let scratch = Scratch::new();
    let shop = scratch.path().join("shop");
    fs::create_dir_all(&shop).unwrap();
    fs::write(shop.join(STACK_FILE), single()).unwrap();
    let deep = scratch.path().join("d".repeat(DEEP_STATE_DIR_CHARS));
    fs::create_dir_all(&deep).unwrap();
    let in_deep = |args: &[&str]| {
        let mut all = vec!["-p", shop.to_str().unwrap()];
        all.extend_from_slice(args);
        scratch
            .command(&all)
            .env("XDG_STATE_HOME", &deep)
            .output()
            .unwrap()
    };
    let imported = in_deep(&["import"]);
    assert!(imported.status.success(), "{}", shown(&imported));

    let up = in_deep(&["up"]);

    assert_eq!(
        up.status.code(),
        Some(SERVICE_UNAVAILABLE),
        "{}",
        shown(&up)
    );
    assert!(
        stderr(&up).contains("too long for a Unix socket")
            && stderr(&up).contains("XDG_STATE_HOME")
            && stderr(&up).contains(deep.to_str().unwrap()),
        "{}",
        shown(&up)
    );
}

/// After a service restart, `wait --json` on a process that is not running
/// answers with the cursor `logs --json` gives and the run's last lines,
/// never `o:0`, and a `since` past that end is refused as `logs` refuses it.
#[test]
fn a_wait_after_a_service_restart_answers_from_the_log() {
    let scratch = Scratch::new();
    let shop = scratch.project(
        "shop",
        &stack(&[entry("crash", "echo boom; exit 3", false)]),
    );
    scratch.on(&shop, &["start", "crash"]);
    let (_, crashed) = scratch.json_on(&shop, &["wait", "crash", "--state", "crashed"]);
    assert_eq!(crashed["status"], "matched", "{crashed}");
    let (stopped, _) = scratch.json(&["service", "stop"]);
    assert!(stopped.status.success(), "{}", shown(&stopped));
    let (_, logs) = scratch.json_on(&shop, &["logs", "crash", "-n", "5"]);
    let end = logs["cursor"].as_str().unwrap().to_string();
    assert_ne!(end, "o:0", "{logs}");

    let (_, waited) = scratch.json_on(&shop, &["wait", "crash", "--state", "crashed"]);
    let (_, missed) = scratch.json_on(&shop, &["wait", "crash", "--log", "zzz", "--since", &end]);
    let past = format!(
        "o:{}",
        end.trim_start_matches("o:").parse::<u64>().unwrap() + 1
    );
    let (refused, _) = scratch.json_on(&shop, &["wait", "crash", "--log", "zzz", "--since", &past]);

    assert_eq!(waited["cursor"], end.as_str(), "{waited}");
    assert_eq!(missed["status"], "not-running", "{missed}");
    assert_eq!(missed["cursor"], end.as_str(), "{missed}");
    assert_eq!(missed["tail"], serde_json::json!(["boom"]), "{missed}");
    assert_eq!(
        refused.status.code(),
        Some(INVALID_INPUT),
        "{}",
        shown(&refused)
    );
    assert!(
        stdout(&refused).contains("past the end"),
        "{}",
        shown(&refused)
    );
}

/// A state directory whose holder sockets fit serves, however long: the
/// service socket lives in the short runtime directory, and the name it is
/// first bound at is no longer than its own.
#[test]
fn a_long_state_dir_whose_sockets_fit_serves() {
    let scratch = Scratch::new();
    let shop = scratch.path().join("shop");
    fs::create_dir_all(&shop).unwrap();
    fs::write(shop.join(STACK_FILE), single()).unwrap();
    let prefix = scratch.path().join("s");
    let padding = LONG_STATE_HOME_CHARS - prefix.as_os_str().len();
    let long = PathBuf::from(format!("{}{}", prefix.display(), "s".repeat(padding)));
    fs::create_dir_all(&long).unwrap();
    let in_long = |args: &[&str]| {
        let mut all = vec!["-p", shop.to_str().unwrap()];
        all.extend_from_slice(args);
        scratch
            .command(&all)
            .env("XDG_STATE_HOME", &long)
            .output()
            .unwrap()
    };
    assert!(in_long(&["import"]).status.success());

    let up = in_long(&["up"]);
    let stopped = in_long(&["service", "stop"]);

    assert!(up.status.success(), "{}", shown(&up));
    assert!(stopped.status.success(), "{}", shown(&stopped));
}

/// A runtime directory too deep for the service socket is refused before a
/// service is started, naming the socket and the variable that moves it.
#[test]
fn a_runtime_dir_too_deep_for_a_socket_is_reported() {
    let scratch = Scratch::new();
    let shop = scratch.project("shop", &single());
    let deep = scratch.path().join("r".repeat(DEEP_STATE_DIR_CHARS));

    let up = scratch
        .command(&["-p", shop.to_str().unwrap(), "up"])
        .env("XDG_RUNTIME_DIR", &deep)
        .output()
        .unwrap();

    assert_eq!(
        up.status.code(),
        Some(SERVICE_UNAVAILABLE),
        "{}",
        shown(&up)
    );
    assert!(
        stderr(&up).contains("too long for a Unix socket")
            && stderr(&up).contains("XDG_RUNTIME_DIR")
            && stderr(&up).contains(deep.to_str().unwrap()),
        "{}",
        shown(&up)
    );
}

/// `-p` naming a folder inside a project acts on that project, and
/// `MUSTER_PROJECT` takes a registered project's name as well as a folder.
#[test]
fn a_subfolder_and_a_project_name_resolve_to_the_project() {
    let scratch = Scratch::new();
    let shop = scratch.project("shop", &single());
    let nested = shop.join("src/deep");
    fs::create_dir_all(&nested).unwrap();

    let (noted, _) = scratch.json_on(&nested, &["note", "set", "plan", "ship it"]);
    assert!(noted.status.success(), "{}", shown(&noted));
    let (read, note) = scratch.json_on(&shop, &["note", "get", "plan"]);
    assert!(read.status.success(), "{}", shown(&read));
    assert_eq!(note["body"], "ship it");

    let elsewhere = scratch.path().join("elsewhere");
    fs::create_dir_all(&elsewhere).unwrap();
    let named = scratch
        .command(&["note", "get", "plan", "--json"])
        .current_dir(&elsewhere)
        .env("MUSTER_PROJECT", "shop")
        .output()
        .unwrap();
    assert!(named.status.success(), "{}", shown(&named));
    assert_eq!(parsed(&named)["body"], "ship it");
}

/// The coordination commands and `doctor` answer `--json` with snake_case
/// keys and kebab-case values, and `doctor` exits non-zero on a failed check
/// either way.
#[test]
fn coordination_and_doctor_answer_json() {
    let scratch = Scratch::new();
    let shop = scratch.project("shop", &single());
    let json = |args: &[&str]| {
        let (output, value) = scratch.json_on(&shop, args);
        assert!(output.status.success(), "{args:?}: {}", shown(&output));
        value
    };

    let rfc3339 = |at: &Value| {
        at.as_str()
            .is_some_and(|at| humantime::parse_rfc3339(at).is_ok())
    };

    assert_eq!(json(&["note", "set", "plan", "body"])["saved"], "plan");
    let notes = json(&["note"]);
    assert_eq!(notes["notes"][0]["key"], "plan");
    assert!(rfc3339(&notes["notes"][0]["updated_at"]), "{notes}");
    let note = json(&["note", "get", "plan"]);
    assert!(rfc3339(&note["updated_at"]), "{note}");
    assert_eq!(
        json(&["note", "rm", "plan"]),
        serde_json::json!({"key": "plan", "deleted": true})
    );
    let added = json(&["todo", "add", "ship"]);
    assert_eq!(added["status"], "pending");
    assert!(rfc3339(&added["updated_at"]), "{added}");
    let id = added["id"].as_str().unwrap().to_string();
    assert_eq!(
        json(&["todo", "status", &id, "in-progress"])["status"],
        "in-progress"
    );
    assert_eq!(json(&["todo"])["todos"][0]["id"], id.as_str());
    assert_eq!(json(&["kv", "set", "port", "3000"])["saved"], "port");
    let value = json(&["kv", "get", "port"]);
    assert_eq!(value["value"], "3000");
    assert!(rfc3339(&value["updated_at"]), "{value}");
    let entries = json(&["kv"]);
    assert_eq!(entries["entries"][0]["key"], "port");
    assert!(rfc3339(&entries["entries"][0]["updated_at"]), "{entries}");
    assert_eq!(json(&["lock"])["locks"], serde_json::json!([]));
    assert_eq!(json(&["lock", "release", "db"])["released"], false);
    let snapshot = scratch.path().join("coordination.yml");
    let exported = json(&["coordination", "export", snapshot.to_str().unwrap()]);
    assert_eq!(exported["todos"], 1);
    assert_eq!(exported["values"], 1);

    let unknown = scratch.on(&shop, &["note", "get", "ghost", "--json"]);
    assert_eq!(
        unknown.status.code(),
        Some(INVALID_INPUT),
        "{}",
        shown(&unknown)
    );
    assert_eq!(
        parsed(&unknown)["error"]["kind"],
        "unknown-note",
        "{}",
        shown(&unknown)
    );

    let (healthy, report) = scratch.json(&["doctor"]);
    assert!(healthy.status.success(), "{}", shown(&healthy));
    assert_eq!(report["failed"], false, "{report}");
    assert!(
        report["checks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|check| check["check"] == "this-folder"),
        "{report}"
    );

    Connection::open(scratch.database())
        .unwrap()
        .execute("UPDATE processes SET stop_grace_ms = 7200000", [])
        .unwrap();
    let (broken, report) = scratch.json(&["doctor"]);
    assert_eq!(
        broken.status.code(),
        Some(ACTION_FAILED),
        "{}",
        shown(&broken)
    );
    assert_eq!(report["failed"], true, "{report}");
}

/// `logs -f --json` prints one object per line, each naming its process
/// and carrying the cursor just past it.
#[test]
fn a_followed_log_in_json_carries_cursors() {
    let scratch = Scratch::new();
    let ticker = scratch.project("tick", &ticking());
    let up = scratch.on(&ticker, &["up"]);
    assert!(up.status.success(), "{}", shown(&up));
    let mut follow = scratch
        .command(&[
            "-p",
            ticker.to_str().unwrap(),
            "logs",
            "-f",
            "ticker",
            "--json",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(follow.stdout.take().unwrap()).lines();
    let first: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    let second: Value = serde_json::from_str(&lines.next().unwrap().unwrap()).unwrap();
    let _ = follow.kill();
    let _ = follow.wait();

    assert_eq!(first["process"], "ticker", "{first}");
    assert_eq!(first["text"], "tick", "{first}");
    let offset = |line: &Value| {
        line["cursor"].as_str().unwrap()["o:".len()..]
            .parse::<u64>()
            .unwrap()
    };
    assert!(offset(&second) > offset(&first), "{first} {second}");
}

/// Neither `/` nor the home folder becomes a project: `add`, `import` and
/// `init --yes` there refuse and say how to pick one.
#[test]
fn the_filesystem_root_and_home_are_never_registered() {
    let scratch = Scratch::new();
    let home = scratch.path();
    fs::write(home.join(STACK_FILE), single()).unwrap();
    let from = |folder: &Path, args: &[&str]| {
        scratch
            .command(args)
            .current_dir(folder)
            .env("HOME", &home)
            .output()
            .unwrap()
    };

    for (folder, why) in [
        (Path::new("/"), "the filesystem root"),
        (home.as_path(), "your home folder"),
    ] {
        for args in [
            &["add", "web", "--", "true"][..],
            &["import"][..],
            &["init", "--yes"][..],
        ] {
            let refused = from(folder, args);
            assert!(!refused.status.success(), "{args:?}: {}", shown(&refused));
            assert!(
                stderr(&refused).contains(why) && stderr(&refused).contains("pass -p"),
                "{args:?}: {}",
                shown(&refused)
            );
        }
    }
    let (_, projects) = scratch.json(&["projects"]);
    assert_eq!(projects, serde_json::json!([]), "{projects}");
}
