//! The shell hook end to end: bash, zsh and fish, each with no rc file and
//! muster's hook loaded as a person's rc file would, typed at on a
//! pseudo-terminal over isolated XDG roots, running the built binary. A
//! shell that is not installed is skipped with a printed reason.

use std::{
    fs,
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::Output,
    time::{Duration, Instant},
};

use muster::{
    adapter::{
        service::SHELL_SWEEP_INTERVAL,
        shell::{HookShell, SHELL_REPORT_SUBCOMMAND},
    },
    domain::{occupancy::ReportSeq, value::ProjectRoot},
};
use rustix::process::{Pid, Signal, kill_process};
use serde_json::Value;
use strum::IntoEnumIterator;

use crate::common::{
    Scratch, ShellSession, WAIT, entry, on_path, processes_under, runnable_shell, stack, wait_until,
};

/// How many times a prompt fits in a report blocked on the frozen service:
/// loose enough for a loaded box, while a hook that waited would take the
/// whole block at every prompt.
const PROMPTS_PER_BLOCK: u32 = 2;
/// The seq of the report timed against the frozen service, newer than any
/// the hook sends, so it is never skipped as stale.
const BLOCKED_REPORT_SEQ: u64 = u64::MAX;
/// Prompts timed while the service is frozen.
const FROZEN_PROMPTS: usize = 20;
/// The idle stop the leaving tests set, as the CLI reads it.
const SHORT_IDLE_STOP: &str = "1s";
/// That idle stop as a duration.
const SHORT_IDLE_STOP_TIME: Duration = Duration::from_secs(1);
/// Slack past the sweep and the idle stop for a stop to land on a loaded
/// box: one more sweep, short of the next sweep but one.
const STOP_SLACK: Duration = SHELL_SWEEP_INTERVAL;
/// Prompts typed in the tests that count reports or look for noise.
const PROMPTS: u64 = 5;
/// Prompts a seq read may miss: the one after the last marker, whose hook
/// may not have run yet when its reports are found done.
const UNSEEN_PROMPTS: u64 = 1;
/// The stack file `import` reads.
const STACK_FILE: &str = "muster.yml";
/// The subcommand the service runs as.
const SERVICE_SUBCOMMAND: &str = "__service";
/// What a job-control message carries in bash, zsh or fish.
const JOB_NOISE: [&str; 4] = ["[1]", "Done", "Job", "has ended"];
/// The program that holds a port in another project.
const PERL: &str = "perl";

/// Runs `body` once for each shell installed, printing which are skipped.
fn each_shell(test: &str, body: impl Fn(&mut Scratch, HookShell, &Path)) {
    for shell in HookShell::iter() {
        let mut scratch = Scratch::new();
        let Some(program) = runnable_shell(&scratch, shell) else {
            eprintln!("{shell} is not installed or does not run: {test} is skipped for it");
            continue;
        };
        body(&mut scratch, shell, &program);
    }
}

/// The project in `folder` of `scratch`, holding the processes of `stack`.
fn project(scratch: &Scratch, folder: &str, stack: &str) -> PathBuf {
    let dir = scratch.path().join(folder);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(STACK_FILE), stack).unwrap();
    succeeded(&scratch.muster(&["-p", dir.to_str().unwrap(), "import"]));
    dir
}

/// A project whose `api` starts on entering and whose `tool` does not.
fn shop(scratch: &Scratch) -> PathBuf {
    project(
        scratch,
        "shop",
        &stack(&[
            entry("api", "sleep 600", true),
            entry("tool", "sleep 600", false),
        ]),
    )
}

/// Fails the test, showing `output`, unless it succeeded.
fn succeeded(output: &Output) {
    assert!(
        output.status.success(),
        "status {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// `ps --json` of `project`.
fn ps(scratch: &Scratch, project: &Path) -> Value {
    let output = scratch.muster(&["-p", project.to_str().unwrap(), "ps", "--json"]);
    succeeded(&output);
    serde_json::from_slice(&output.stdout).unwrap()
}

/// The `ps --json` row of `name` in `project`.
fn row(scratch: &Scratch, project: &Path, name: &str) -> Value {
    let statuses = ps(scratch, project);
    statuses[0]["processes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|process| process["name"] == name)
        .cloned()
        .unwrap_or_else(|| panic!("no row {name}: {statuses}"))
}

/// Whether `name` of `project` is running.
fn running(scratch: &Scratch, project: &Path, name: &str) -> bool {
    row(scratch, project, name)["state"] == "running"
}

/// Waits until the shell's record says it is in `project`, which it writes
/// only after leaving the notes the service answered.
fn wait_recorded_in(session: &ShellSession, project: &Path) {
    let root = ProjectRoot::try_new(project.to_path_buf()).unwrap();
    wait_until("the shell's report to be recorded", || {
        session
            .record()
            .is_some_and(|record| record.project().as_ref() == Some(&root))
    });
}

/// Enters `project` in `session` and waits until its `api` runs and the
/// report that started it is recorded.
fn enter(scratch: &Scratch, session: &ShellSession, project: &Path) {
    session.cd(project);
    wait_until("entering to start api", || running(scratch, project, "api"));
    wait_recorded_in(session, project);
}

/// The one service running over `scratch`.
fn service_pid(scratch: &Scratch) -> Pid {
    let services = processes_under(SERVICE_SUBCOMMAND, scratch.root());
    assert_eq!(services.len(), 1, "{services:?}");
    Pid::from_raw(services[0] as i32).unwrap()
}

/// Entering a project starts its autostart processes and only those, and
/// the prompt after the report says what started.
#[test]
fn entering_a_project_starts_autostart_and_the_next_prompt_says_so() {
    each_shell("entering", |scratch, shell, program| {
        let shop = shop(scratch);
        let session = ShellSession::start(scratch, shell, program);

        enter(scratch, &session, &shop);
        session.settle();

        session.wait_drawn("muster: shop: starting api");
        assert!(
            !session.drawn().contains("starting api, tool"),
            "{shell}: {}",
            session.drawn()
        );
        assert!(!running(scratch, &shop, "tool"), "{shell}");
    });
}

/// How long a report run in the foreground blocks on the frozen service,
/// for the shell `session`: what every prompt would cost a hook that waited.
fn blocked_report(scratch: &Scratch, session: &ShellSession) -> Duration {
    let pid = session.pid().to_string();
    let seq = BLOCKED_REPORT_SEQ.to_string();
    let started = Instant::now();
    scratch.muster(&[SHELL_REPORT_SUBCOMMAND, "--pid", &pid, "--seq", &seq]);
    let blocked = started.elapsed();
    assert_ne!(
        session.record().map(|record| *record.seq()),
        Some(ReportSeq::new(BLOCKED_REPORT_SEQ)),
        "a report to a frozen service cannot land"
    );
    blocked
}

/// With the service stopped dead, every prompt still returns in a fraction
/// of the time a report blocks on it: the hook never waits on the service.
#[test]
fn the_prompt_returns_while_the_service_is_frozen() {
    each_shell("frozen", |scratch, shell, program| {
        let shop = shop(scratch);
        let session = ShellSession::start(scratch, shell, program);
        enter(scratch, &session, &shop);
        let service = service_pid(scratch);

        kill_process(service, Signal::STOP).unwrap();
        let prompts: Vec<Duration> = (0..FROZEN_PROMPTS).map(|_| session.round_trip()).collect();
        let budget = blocked_report(scratch, &session) / PROMPTS_PER_BLOCK;
        kill_process(service, Signal::CONT).unwrap();

        assert!(
            prompts.iter().all(|prompt| *prompt < budget),
            "{shell}: each prompt under {budget:?}: {prompts:?}"
        );
    });
}

/// Leaving the project arms its idle stop, which stops it when due.
#[test]
fn leaving_stops_the_stack_after_idle_stop() {
    each_shell("leaving", |scratch, shell, program| {
        succeeded(&scratch.muster(&["config", "set", "idle-stop", SHORT_IDLE_STOP]));
        let shop = shop(scratch);
        let session = ShellSession::start(scratch, shell, program);
        enter(scratch, &session, &shop);

        session.cd(Path::new("/"));

        wait_until("idle stop to stop api", || !running(scratch, &shop, "api"));
        let api = row(scratch, &shop, "api");
        assert_eq!(api["state"], "exited", "{shell}: {api}");
    });
}

/// A shell that dies inside the project leaves it once the sweep finds it
/// gone, and the idle stop follows.
#[test]
fn closing_the_shell_counts_as_leaving() {
    each_shell("closing", |scratch, shell, program| {
        succeeded(&scratch.muster(&["config", "set", "idle-stop", SHORT_IDLE_STOP]));
        let shop = shop(scratch);
        let mut session = ShellSession::start(scratch, shell, program);
        enter(scratch, &session, &shop);

        session.kill();
        let closed = Instant::now();
        wait_until("the sweep and idle stop to stop api", || {
            !running(scratch, &shop, "api")
        });

        let bound = SHELL_SWEEP_INTERVAL + STOP_SLACK + SHORT_IDLE_STOP_TIME;
        assert!(
            closed.elapsed() < bound,
            "{shell}: stopped after {:?}",
            closed.elapsed()
        );
    });
}

/// A service restarted while a shell sits in a running project rebuilds
/// its occupancy from the shell's record: one shell, no idle stop pending,
/// and nothing started again.
#[test]
fn a_service_restart_keeps_the_shell_s_occupancy() {
    each_shell("restart", |scratch, shell, program| {
        let shop = shop(scratch);
        let session = ShellSession::start(scratch, shell, program);
        enter(scratch, &session, &shop);
        let pid = row(scratch, &shop, "api")["pid"].clone();

        succeeded(&scratch.muster(&["service", "stop"]));

        wait_until("the new service to adopt api", || {
            let api = row(scratch, &shop, "api");
            api["state"] == "running" && api["pid"] == pid
        });
        let statuses = ps(scratch, &shop);
        assert_eq!(statuses[0]["occupancy"]["shells"], 1, "{shell}: {statuses}");
        assert!(
            statuses[0]["occupancy"]["stops_in"].is_null(),
            "{shell}: {statuses}"
        );
        assert!(!running(scratch, &shop, "tool"), "{shell}");
    });
}

/// The hook hands the last command's exit status on to the next one.
#[test]
fn the_exit_status_survives_the_hook() {
    each_shell("exit status", |scratch, shell, program| {
        let session = ShellSession::start(scratch, shell, program);

        session.type_line("false");
        let expected = session.echo_status(1);

        session.wait_drawn(&expected);
    });
}

/// Entering, leaving and many prompts, with every report finished, draw
/// no job-control message.
#[test]
fn the_hook_prints_no_job_control_noise() {
    each_shell("noise", |scratch, shell, program| {
        let shop = shop(scratch);
        let session = ShellSession::start(scratch, shell, program);
        enter(scratch, &session, &shop);
        session.cd(Path::new("/"));
        for _ in 0..PROMPTS {
            session.settle();
        }

        session.wait_reports_done();
        session.settle();
        session.settle();

        let drawn = session.drawn();
        for noise in JOB_NOISE {
            assert!(!drawn.contains(noise), "{shell} drew {noise:?}: {drawn}");
        }
    });
}

/// A hook loaded a second time is not added again: each prompt reports
/// once, so the reported seq grows by one per prompt, give or take the
/// prompt each reading may run ahead of; a hook added twice grows it twice
/// as fast.
#[test]
fn loading_the_hook_twice_reports_once_per_prompt() {
    each_shell("loading twice", |scratch, shell, program| {
        let session = ShellSession::start(scratch, shell, program);
        session.load_hook();
        session.wait_reports_done();
        let before = session.record().unwrap().seq().into_inner();

        for _ in 0..PROMPTS {
            session.settle();
        }
        session.wait_reports_done();

        let after = session.record().unwrap().seq().into_inner();
        let grown = after - before;
        assert!(
            (PROMPTS - UNSEEN_PROMPTS..=PROMPTS + UNSEEN_PROMPTS).contains(&grown),
            "{shell}: {before} then {after} over {PROMPTS} prompts"
        );
    });
}

/// An autostart process whose declared port another project's process
/// holds is refused on entering, and the next prompt says so with the
/// command that frees it.
#[test]
fn a_blocked_autostart_is_printed_with_its_hint() {
    let Some(perl) = on_path(PERL) else {
        eprintln!("{PERL} is not installed: blocked autostart is skipped");
        return;
    };
    each_shell("blocked autostart", |scratch, shell, program| {
        let port = free_port();
        let listen = format!(
            "exec {} -MIO::Socket::INET -e 'my $listener = IO::Socket::INET->new(LocalAddr => \"127.0.0.1\", LocalPort => {port}, Listen => 1, ReuseAddr => 1) or die $!; sleep 600'",
            perl.display()
        );
        let admin = project(scratch, "admin", &stack(&[entry("holder", &listen, true)]));
        succeeded(&scratch.muster(&["-p", admin.to_str().unwrap(), "up"]));
        wait_until("admin to hold the port", || {
            TcpStream::connect(("127.0.0.1", port)).is_ok()
        });
        let shop = project(scratch, "shop", &stack(&[entry("web", "sleep 600", true)]));
        succeeded(&scratch.muster(&[
            "-p",
            shop.to_str().unwrap(),
            "set",
            "web",
            "--port",
            &port.to_string(),
        ]));
        let session = ShellSession::start(scratch, shell, program);

        session.cd(&shop);

        let hint = "`muster down -p admin` frees it";
        let started = Instant::now();
        while !session.drawn().contains(hint) {
            assert!(started.elapsed() < WAIT, "{shell}: {}", session.drawn());
            session.settle();
        }
        let drawn = session.drawn();
        let line = drawn
            .split(['\r', '\n'])
            .find(|line| line.contains(hint))
            .unwrap_or_default();
        assert!(
            line.starts_with(&format!("muster: shop: web did not start: port {port}")),
            "{shell}: {line}"
        );
        assert!(!running(scratch, &shop, "web"), "{shell}");
    });
}

/// A localhost port free at the moment of asking.
fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
