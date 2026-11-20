//! `muster view` end to end: the built binary serving its page on loopback
//! over isolated state, answering only requests under its token's path,
//! never starting the service, and ending as each signal asks.

use std::{
    fs,
    io::{BufRead, BufReader, Read},
    net::{Ipv4Addr, TcpListener},
    os::unix::process::ExitStatusExt,
    path::{Path, PathBuf},
    process::Stdio,
    thread,
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, unbounded};
use muster_os::test_script::runnable_script;
use muster_protocol::service::{ServiceRequest, ServiceResponse, endpoint::SERVICE_SUBCOMMAND};
use muster_web::{EventReader, SseEvent, exchange, get};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use rusqlite::Connection;
use rustix::process::{Pid, Signal, test_kill_process};
use serde_json::Value;

use crate::common::{Entry, Scratch, ViewSession, WAIT, entry, processes_under, stack, wait_until};

/// What the view prints after its address.
const STOP_HINT: &str = "Press Ctrl-C to stop.";
/// The variable naming the browser the view opens.
const BROWSER_ENV: &str = "BROWSER";
/// A browser that does not exist, so opening the page fails.
const MISSING_BROWSER: &str = "no-such-browser";
/// How the warning that the browser did not open starts.
const NO_BROWSER_WARNING: &str = "▲ could not open a browser: ";
/// How that warning ends.
const NO_BROWSER_HINT: &str = "; open the address above";
/// The page's Content Security Policy, as the spec writes it.
const POLICY: &str = "default-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'";
/// The code a Ctrl-C ends a command with.
const INTERRUPTED: i32 = 130;
/// How long the view runs before a test checks it started nothing.
const SETTLE: Duration = Duration::from_secs(3);
/// The pause between the page loads a settling test makes.
const LOOK_PAUSE: Duration = Duration::from_millis(500);
/// A process name a page would run as markup if it ever set it as HTML.
const HOSTILE_NAME: &str = "<img src=x onerror=alert(1)>";
/// The markup the hostile process prints.
const HOSTILE_LINE: &str = "<script>alert(1)</script>";
/// The hostile process: it prints markup, then clears the screen, then
/// waits.
const HOSTILE_COMMAND: &str = r"printf '<script>alert(1)</script>\n\033[2J'; sleep 600";
/// A command that runs until it is stopped.
const IDLE: &str = "sleep 600";
/// The escape character, which no log tail line may carry.
const ESCAPE: char = '\u{1b}';
/// The stand-in browser a test has the view open, and where it writes its
/// pid.
const BROWSER_STAND_IN: &str = "browser.sh";
const BROWSER_PID_FILE: &str = "browser.pid";
/// The pragma holding the database's schema version.
const USER_VERSION: &str = "user_version";
/// A schema version an older muster wrote, with migrations still pending.
const OLDER_SCHEMA: u32 = 1;

/// Registers the project `name` holding `entries` in a folder of its own
/// under `scratch`, and returns that folder.
fn register(scratch: &Scratch, name: &str, entries: &[Entry<'_>]) -> PathBuf {
    let project = scratch.path().join(name);
    fs::create_dir_all(&project).unwrap();
    fs::write(project.join("muster.yml"), stack(entries)).unwrap();
    let import = scratch.muster(&["-p", project.to_str().unwrap(), "import"]);
    assert!(import.status.success(), "{import:?}");
    project
}

/// Starts process `name` of the project in `project`, starting the
/// service.
fn start(scratch: &Scratch, project: &Path, name: &str) {
    let started = scratch.muster(&["-p", project.to_str().unwrap(), "start", name]);
    assert!(started.status.success(), "{started:?}");
}

/// Whether `event` is a snapshot that shows the service as `health` and the
/// projects read.
fn settled(event: &SseEvent, health: &str) -> bool {
    event.kind() == "snapshot"
        && event.data()["health"] == health
        && event.data()["projects"]
            .as_array()
            .is_some_and(|projects| !projects.is_empty())
}

/// The first snapshot on `events` that is [`settled`], which a page that
/// connects before the view's first readings waits for.
fn settled_snapshot(events: &mut EventReader, health: &str) -> SseEvent {
    events.event_where(&format!("a snapshot with the service {health}"), |event| {
        settled(event, health)
    })
}

/// The names of the processes of the first project `snapshot` holds.
fn process_names(snapshot: &SseEvent) -> Vec<String> {
    names_in(&snapshot.data()["projects"])
}

/// The names of the processes of the first of `projects`.
fn names_in(projects: &Value) -> Vec<String> {
    projects[0]["processes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|process| process["name"].as_str().unwrap().to_string())
        .collect()
}

/// Whether `event` is about process `name`, as a message of `kind`.
fn about(event: &SseEvent, kind: &str, name: &str) -> bool {
    event.kind() == kind
        && (event.data()["name"] == name || event.data()["process"]["name"] == name)
}

/// The stream opens with a snapshot: with shop's `web` and `worker`
/// registered and no service, its first event is a snapshot, and once the
/// view has read, the floor the page builds names both, the service absent.
/// A page that connects before the first projects reading gets them as a
/// `projects` message after its snapshot: with no service, no status answer
/// follows to send them as a snapshot again.
#[test]
fn the_stream_opens_with_a_snapshot_of_the_stack() {
    let mut scratch = Scratch::new();
    register(&scratch, "shop", &[
        entry("web", IDLE, false),
        entry("worker", IDLE, false),
    ]);
    let view = ViewSession::start(&mut scratch, &[]);

    let mut events = view.events();

    assert_eq!(*events.status(), 200);
    assert_eq!(events.header("Content-Type"), Some("text/event-stream"));
    let first = events.next_event().unwrap();
    assert_eq!(first.kind(), "snapshot");
    let mut health = first.data()["health"].clone();
    let mut projects = first.data()["projects"].clone();
    let read = |health: &Value, projects: &Value| {
        health == "absent" && projects.as_array().is_some_and(|read| !read.is_empty())
    };
    if !read(&health, &projects) {
        events.event_where("the projects read with the service absent", |event| {
            match event.kind().as_str() {
                "snapshot" => {
                    health = event.data()["health"].clone();
                    projects = event.data()["projects"].clone();
                },
                "projects" => projects = event.data()["projects"].clone(),
                _ => {},
            }
            read(&health, &projects)
        });
    }
    assert_eq!(names_in(&projects), ["web", "worker"]);
}

/// Starting a process streams its new room and an ok pin: the pin goes out
/// as its start event is heard, so it may come before or after the room.
#[test]
fn starting_a_process_streams_its_new_room() {
    let mut scratch = Scratch::new();
    let shop = register(&scratch, "shop", &[
        entry("web", IDLE, false),
        entry("worker", IDLE, false),
    ]);
    start(&scratch, &shop, "worker");
    let view = ViewSession::start(&mut scratch, &[]);
    let mut events = view.events();
    settled_snapshot(&mut events, "running");

    start(&scratch, &shop, "web");

    let mut rooms = Vec::new();
    let pin = events.event_where("web's pin", |event| {
        if about(event, "process", "web") {
            rooms.push(event.data()["process"]["room"].clone());
        }
        about(event, "pin", "web")
    });
    if rooms.is_empty() {
        let moved = events.event_where("web's new room", |event| about(event, "process", "web"));
        rooms.push(moved.data()["process"]["room"].clone());
    }
    assert_eq!(pin.data()["pin"], "ok");
    let room = &rooms[0];
    assert!(room == "starting" || room == "running", "{room}");
}

/// Hostile names and log lines arrive as plain strings: a process named as
/// markup that prints markup then clears the screen shows in the snapshot
/// under that exact name, and its log tail, asked by its percent-encoded
/// name, holds the markup line with no escape character.
#[test]
fn hostile_names_and_log_lines_arrive_as_plain_strings() {
    let mut scratch = Scratch::new();
    let shop = register(&scratch, "shop", &[entry(
        HOSTILE_NAME,
        HOSTILE_COMMAND,
        false,
    )]);
    start(&scratch, &shop, HOSTILE_NAME);
    let view = ViewSession::start(&mut scratch, &[]);

    let snapshot = settled_snapshot(&mut view.events(), "running");
    let route = format!(
        "api/log?project=shop&name={}",
        utf8_percent_encode(HOSTILE_NAME, NON_ALPHANUMERIC)
    );
    let mut lines = Vec::new();
    wait_until("the markup line in the log tail", || {
        let tail = view.get(&route);
        assert_eq!(tail.status(), &200, "{}", tail.text());
        let body: Value = serde_json::from_slice(tail.body()).unwrap();
        lines = body["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|line| line.as_str().unwrap().to_string())
            .collect();
        lines.iter().any(|line| line == HOSTILE_LINE)
    });

    assert_eq!(process_names(&snapshot), [HOSTILE_NAME]);
    assert!(lines.iter().all(|line| !line.contains(ESCAPE)), "{lines:?}");
}

/// Looking never loads a project: with shop and blog registered and only
/// shop's `web` started, after a few seconds of a page streaming and
/// reading blog's log tail, the service has loaded only shop.
#[test]
fn looking_never_loads_a_project() {
    let mut scratch = Scratch::new();
    let shop = register(&scratch, "shop", &[entry("web", IDLE, false)]);
    register(&scratch, "blog", &[entry("api", IDLE, false)]);
    start(&scratch, &shop, "web");
    let view = ViewSession::start(&mut scratch, &[]);
    let mut events = view.events();
    settled_snapshot(&mut events, "running");

    let started = Instant::now();
    while started.elapsed() < SETTLE {
        assert_eq!(view.get("api/log?project=blog&name=api").status(), &200);
        thread::sleep(LOOK_PAUSE);
    }

    let ServiceResponse::Status(loaded) = scratch.send(&ServiceRequest::Ps { project: None })
    else {
        panic!("ps answers a status");
    };
    let names: Vec<String> = loaded
        .projects()
        .iter()
        .map(|status| status.project().name().to_string())
        .collect();
    assert_eq!(names, ["shop"]);
}

/// The view answers only under its token's path: the bare root, a wrong
/// token and a cookie are forbidden, the printed address opens the shell
/// with the exact policy and this version's entry module, and a foreign
/// host or a POST is refused.
#[test]
fn the_view_answers_only_with_its_token() {
    let mut scratch = Scratch::new();
    let view = ViewSession::start(&mut scratch, &[]);
    let port = view.port();
    let root = format!("/v/{}/", view.token());

    assert_eq!(view.next_line(), STOP_HINT);
    assert_eq!(view.token().len(), 64, "{}", view.url());
    assert_eq!(view.url(), format!("http://127.0.0.1:{port}{root}"));

    let cookie = format!("muster-view-{port}={}", view.token());
    assert_eq!(get(port, "/", &[]).status(), &403);
    assert_eq!(get(port, "/", &[("Cookie", &cookie)]).status(), &403);
    assert_eq!(
        get(port, &format!("/v/{}/", "0".repeat(64)), &[]).status(),
        &403
    );

    let shell = get(port, &root, &[]);
    assert_eq!(shell.status(), &200);
    assert_eq!(shell.header("Content-Security-Policy"), Some(POLICY));
    assert_eq!(shell.header("Set-Cookie"), None);
    let entry_module = format!("assets/v{}/js/app.js", env!("CARGO_PKG_VERSION"));
    assert!(shell.text().contains(&entry_module), "{}", shell.text());
    assert_eq!(
        get(port, &format!("{root}{entry_module}"), &[]).status(),
        &200
    );

    let foreign = exchange(
        port,
        format!("GET {root} HTTP/1.1\r\nHost: evil.example:{port}\r\n\r\n").as_bytes(),
    );
    assert_eq!(foreign.status(), &403);

    let post = exchange(
        port,
        format!("POST {root} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: 0\r\n\r\n")
            .as_bytes(),
    );
    assert_eq!(post.status(), &405);
}

/// The view listens on the port it is given.
#[test]
fn the_view_listens_on_the_port_it_is_given() {
    let mut scratch = Scratch::new();
    let free = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port();

    let view = ViewSession::start(&mut scratch, &["--port", &free.to_string()]);

    assert_eq!(view.port(), free);
    assert_eq!(get(free, "/", &[]).status(), &403);
}

/// Looking never starts the service: while a page loads the view again and
/// again against a registered project, no service runs and no socket exists.
#[test]
fn the_view_never_starts_the_service() {
    let mut scratch = Scratch::new();
    let project = scratch.path().join("shop");
    fs::create_dir_all(&project).unwrap();
    fs::write(
        project.join("muster.yml"),
        stack(&[entry("api", "sleep 600", true)]),
    )
    .unwrap();
    let import = scratch.muster(&["-p", project.to_str().unwrap(), "import"]);
    assert!(import.status.success(), "{import:?}");
    let mut view = ViewSession::start(&mut scratch, &[]);
    let root = format!("/v/{}/", view.token());

    let started = Instant::now();
    while started.elapsed() < SETTLE {
        assert_eq!(get(view.port(), &root, &[]).status(), &200);
        thread::sleep(LOOK_PAUSE);
    }

    assert!(view.running());
    assert!(processes_under(SERVICE_SUBCOMMAND, scratch.root()).is_empty());
    assert!(!scratch.socket().exists());
}

/// Ctrl-C ends the view as interrupted.
#[test]
fn ctrl_c_ends_the_view_as_interrupted() {
    let mut scratch = Scratch::new();
    let mut view = ViewSession::start(&mut scratch, &[]);

    let status = view.signal(Signal::INT);

    assert_eq!(status.code(), Some(INTERRUPTED), "{status:?}");
}

/// A terminate or hangup ends the view by that signal, so its parent sees
/// the signal, as `attach` does.
#[test]
fn a_terminate_or_hangup_ends_the_view_by_that_signal() {
    for signal in [Signal::TERM, Signal::HUP] {
        let mut scratch = Scratch::new();
        let mut view = ViewSession::start(&mut scratch, &[]);

        let status = view.signal(signal);

        assert_eq!(status.signal(), Some(signal.as_raw()), "{status:?}");
    }
}

/// A browser the view starts outlives the Ctrl-C that ends it: a terminal
/// sends that to the view's whole process group, and the browser runs in a
/// group of its own.
#[test]
fn a_browser_the_view_starts_outlives_its_ctrl_c() {
    let mut scratch = Scratch::new();
    let pid_file = scratch.path().join(BROWSER_PID_FILE);
    let browser = scratch.path().join(BROWSER_STAND_IN);
    runnable_script(
        &browser,
        &format!("echo $$ > '{}'\nexec sleep 600\n", pid_file.display()),
    );
    let mut view = ViewSession::opening_in(&mut scratch, &browser);
    let mut opened = String::new();
    wait_until("the browser to start", || {
        opened = fs::read_to_string(&pid_file).unwrap_or_default();
        opened.ends_with('\n')
    });
    let opened: u32 = opened.trim().parse().unwrap();
    scratch.keep_child(opened);

    let status = view.signal_group(Signal::INT);

    assert_eq!(status.code(), Some(INTERRUPTED), "{status:?}");
    let pid = Pid::from_raw(i32::try_from(opened).unwrap()).unwrap();
    assert!(
        test_kill_process(pid).is_ok(),
        "the browser died with the view"
    );
}

/// A browser that cannot be run is a marked warning on stderr, after the
/// address and how to stop the view, which stay on stdout.
#[test]
fn a_browser_that_fails_is_a_warning_on_stderr() {
    let mut scratch = Scratch::new();
    let mut view = scratch
        .command(&["view"])
        .env(BROWSER_ENV, scratch.path().join(MISSING_BROWSER))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    scratch.keep_child(view.id());
    let out = lines_of(view.stdout.take().unwrap());
    let err = lines_of(view.stderr.take().unwrap());

    let url = out.recv_timeout(WAIT).unwrap();
    assert!(url.starts_with("http://127.0.0.1:"), "{url}");
    assert_eq!(out.recv_timeout(WAIT).unwrap(), STOP_HINT);
    let warning = err.recv_timeout(WAIT).unwrap();
    assert!(
        warning.starts_with(NO_BROWSER_WARNING) && warning.ends_with(NO_BROWSER_HINT),
        "{warning}"
    );

    let _ = view.kill();
    let _ = view.wait();
}

/// The lines `stream` carries, sent as each is read.
fn lines_of(stream: impl Read + Send + 'static) -> Receiver<String> {
    let (sender, lines) = unbounded();
    thread::spawn(move || {
        for line in BufReader::new(stream).lines() {
            let Ok(line) = line else { return };
            if sender.send(line).is_err() {
                return;
            }
        }
    });
    lines
}

/// `-p` names the floor the page opens on, as the address's fragment; a
/// project that is not registered is refused before anything listens.
#[test]
fn a_named_project_is_the_floor_the_page_opens_on() {
    let mut scratch = Scratch::new();
    register(&scratch, "shop", &[entry("web", IDLE, false)]);

    let view = ViewSession::for_project(&mut scratch, "shop");
    let refused = scratch.muster(&["-p", "ghost", "view", "--no-open"]);

    assert_eq!(view.fragment(), "#/shop");
    assert!(!refused.status.success(), "{refused:?}");
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("ghost"),
        "{refused:?}"
    );
}

/// A view run by a process muster started, with no `-p`, opens on the
/// project `MUSTER_PROJECT` names, as every other command acts on it, not
/// on the folder it runs in.
#[test]
fn the_project_muster_gives_its_processes_is_the_floor_the_page_opens_on() {
    let mut scratch = Scratch::new();
    register(&scratch, "blog", &[entry("web", IDLE, false)]);
    let shop = register(&scratch, "shop", &[entry("web", IDLE, false)]);

    let view = ViewSession::in_process_of(&mut scratch, &shop);

    assert_eq!(view.fragment(), "#/shop");
}

/// `-p` against a database an older muster wrote names the pending
/// migration and the command that applies it, instead of calling a
/// registered project unknown, and leaves the database as it found it.
#[test]
fn a_named_project_in_an_older_database_names_its_migration() {
    let scratch = Scratch::new();
    register(&scratch, "shop", &[entry("web", IDLE, false)]);
    Connection::open(scratch.database())
        .unwrap()
        .pragma_update(None, USER_VERSION, OLDER_SCHEMA)
        .unwrap();

    let refused = scratch.muster(&["-p", "shop", "view", "--no-open"]);

    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(!refused.status.success(), "{refused:?}");
    assert!(stderr.contains("migration"), "{stderr}");
    assert!(stderr.contains("muster ps"), "{stderr}");
    assert!(!stderr.contains("no registered project"), "{stderr}");
    let version: u32 = Connection::open(scratch.database())
        .unwrap()
        .pragma_query_value(None, USER_VERSION, |row| row.get(0))
        .unwrap();
    assert_eq!(version, OLDER_SCHEMA);
}
