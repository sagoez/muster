//! The real holder round trip: the built `muster` binary launched detached as
//! `__holder`, outliving its runner, adopted by a new one and stopped.

#![cfg(target_os = "linux")]

use std::{
    fs,
    path::{Path, PathBuf},
    process, thread,
    time::{Duration, Instant},
};

use bytesize::ByteSize as RawByteSize;
use crossbeam_channel::{Receiver, Sender, unbounded};
use muster::{
    adapter::{
        holder::{ExecHolderLauncher, HOLDER_SUBCOMMAND, HolderDirs, HolderRecord, HolderRunner},
        open_files::OpenFilesLimit,
        signal_target::SignalTarget,
    },
    domain::{
        color_scheme::ColorScheme,
        port::{OutputSink, ProcessRunner},
        process::{StopPolicy, StopSignal},
        pty::{ExitOutcome, ProcessOutput, PtySize, SpawnRequest},
        value::{ByteSize, CommandLine, ProcessName, StopGrace},
    },
};
use rustix::process::{Pid, Signal, kill_process};
use tempfile::{Builder, TempDir};

/// The muster binary cargo built for this test.
const BINARY: &str = env!("CARGO_BIN_EXE_muster");
/// Bound on every wait; reaching it fails the test.
const WAIT: Duration = Duration::from_secs(10);
/// Interval between checks of a polled condition.
const POLL: Duration = Duration::from_millis(10);
/// Short root for test sockets, inside `sun_path`.
const SHORT_TEMP_ROOT: &str = "/tmp";
/// Prefix of the scratch directory.
const SCRATCH_PREFIX: &str = "mx-";
/// Log cap of the test runners.
const LOG_CAP_MIB: u64 = 4;
/// Scrollback lines of the test runners' holders.
const SCROLLBACK_LINES: usize = 1000;
/// The child the holder runs.
const TICKER: &str = "while :; do echo tick; sleep 0.05; done";
/// What the ticker prints.
const TICK: &str = "tick";
/// Grace of the ticker's stop policy.
const STOP_GRACE: Duration = Duration::from_secs(2);
/// A variable of the command that launched the ticker.
const LAUNCH_ENTRY: (&str, &str) = ("FROM_CLIENT", "client");
/// A variable of the ticker's own.
const OWN_ENTRY: (&str, &str) = ("PORT", "5173");
/// A variable the test process has, which cargo sets, and the ticker must not.
const INHERITED_VAR: &str = "CARGO_PKG_NAME";

/// Forwards a handle's output into a channel.
struct ChannelSink(Sender<ProcessOutput>);

impl OutputSink for ChannelSink {
    fn send(&self, output: ProcessOutput) {
        let _ = self.0.send(output);
    }
}

/// Receives outputs until `wanted` returns true for one, failing at [`WAIT`].
fn receive_until(
    outputs: &Receiver<ProcessOutput>,
    what: &str,
    wanted: impl Fn(&ProcessOutput) -> bool,
) {
    let deadline = Instant::now() + WAIT;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match outputs.recv_timeout(remaining) {
            Ok(output) if wanted(&output) => return,
            Ok(_) => {},
            Err(_) => panic!("timed out waiting for {what}"),
        }
    }
}

/// Whether `output` is a chunk of output, placed in the log, showing a tick.
fn is_tick(output: &ProcessOutput) -> bool {
    matches!(output, ProcessOutput::Placed { bytes, .. } if String::from_utf8_lossy(bytes).contains(TICK))
}

/// Polls `condition` until it holds, failing at [`WAIT`].
fn wait_until(what: &str, mut condition: impl FnMut() -> bool) {
    let started = Instant::now();
    while !condition() {
        assert!(started.elapsed() < WAIT, "timed out waiting for {what}");
        thread::sleep(POLL);
    }
}

/// Pids of holder processes serving a socket under `root`.
fn holders_under(root: &Path) -> Vec<u32> {
    let root = root.to_string_lossy().into_owned();
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .filter(|pid| {
            let cmdline = fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
            let cmdline = String::from_utf8_lossy(&cmdline);
            cmdline.contains(HOLDER_SUBCOMMAND) && cmdline.contains(&root)
        })
        .collect()
}

/// Holder dirs in a scratch directory; dropping it SIGKILLs every child and
/// holder process the test left there, even on panic.
struct Scratch {
    dirs: HolderDirs,
    root: TempDir,
}

impl Scratch {
    fn new() -> Self {
        let root = Builder::new()
            .prefix(SCRATCH_PREFIX)
            .tempdir_in(SHORT_TEMP_ROOT)
            .unwrap();
        let dirs = HolderDirs::builder()
            .sockets(root.path().join("sock"))
            .records(root.path().join("records"))
            .logs(root.path().join("logs"))
            .build();
        Self { dirs, root }
    }

    fn runner(&self) -> HolderRunner {
        let launcher = ExecHolderLauncher::builder()
            .binary(PathBuf::from(BINARY))
            .open_files(OpenFilesLimit::current())
            .build();
        HolderRunner::new(Box::new(launcher), self.dirs.clone())
    }

    fn request(&self) -> SpawnRequest {
        SpawnRequest::builder()
            .process(ProcessName::try_new("ticker").unwrap())
            .command(CommandLine::try_new(TICKER).unwrap())
            .project(Some(self.root.path().to_path_buf()))
            .launch_environment([(LAUNCH_ENTRY.0.into(), LAUNCH_ENTRY.1.into())].into())
            .environment([(OWN_ENTRY.0.into(), OWN_ENTRY.1.into())].into())
            .size(PtySize::try_new(24, 80).unwrap())
            .scrollback(SCROLLBACK_LINES)
            .log_max_size(ByteSize::try_new(RawByteSize::mib(LOG_CAP_MIB)).unwrap())
            .color_scheme(ColorScheme::Dark)
            .stop(
                StopPolicy::builder()
                    .signal(StopSignal::Terminate)
                    .grace_period(StopGrace::try_new(STOP_GRACE).unwrap())
                    .build(),
            )
            .build()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        for record in HolderRecord::load_all(self.dirs.records()) {
            if record.child_pid().into_inner() != process::id() {
                let _ = SignalTarget::Group(record.child_pid()).send(Signal::KILL);
            }
        }
        for holder in holders_under(self.root.path()) {
            if let Some(holder) = Pid::from_raw(holder as i32) {
                let _ = kill_process(holder, Signal::KILL);
            }
        }
    }
}

#[test]
fn an_exec_launched_holder_outlives_its_runner_is_adopted_and_leaves_nothing_behind() {
    let scratch = Scratch::new();
    let runner = scratch.runner();
    let (sender, outputs) = unbounded();
    let handle = runner
        .spawn(scratch.request(), Box::new(ChannelSink(sender)))
        .unwrap();
    receive_until(&outputs, "a tick", is_tick);
    let record = runner.locate(handle.process_id().unwrap()).unwrap();
    assert_ne!(
        record.holder_pid().into_inner(),
        process::id(),
        "the holder is its own process"
    );
    assert_eq!(holders_under(scratch.root.path()), vec![
        record.holder_pid().into_inner()
    ]);
    let environ = fs::read(format!("/proc/{}/environ", record.child_pid())).unwrap();
    let environ: Vec<String> = environ
        .split(|byte| *byte == 0)
        .map(|entry| String::from_utf8_lossy(entry).into_owned())
        .collect();
    for (name, value) in [LAUNCH_ENTRY, OWN_ENTRY] {
        assert!(environ.contains(&format!("{name}={value}")), "{environ:?}");
    }
    assert!(
        !environ
            .iter()
            .any(|entry| entry.starts_with(&format!("{INHERITED_VAR}="))),
        "the holder's own environment stays out: {environ:?}"
    );
    assert_eq!(record.stop().grace_period().into_inner(), STOP_GRACE);
    drop(handle);
    drop(runner);

    let runner = scratch.runner();
    assert_eq!(runner.live_records(), vec![record.clone()]);
    let (sender, outputs) = unbounded();
    let mut adopted = runner
        .adopt(&record, Box::new(ChannelSink(sender)))
        .unwrap();
    assert_eq!(adopted.process_id(), Some(record.child_pid()));
    receive_until(&outputs, "output after adoption", |output| {
        matches!(output, ProcessOutput::Placed { .. })
    });
    receive_until(&outputs, "a live tick after adoption", is_tick);

    adopted.kill().unwrap();
    receive_until(&outputs, "the exit", |output| {
        matches!(output, ProcessOutput::Exited(ExitOutcome::Failed(_)))
    });
    adopted.acknowledge_exit().unwrap();
    wait_until("the holder process to end", || {
        holders_under(scratch.root.path()).is_empty()
    });
    assert!(HolderRecord::load_all(scratch.dirs.records()).is_empty());
}
