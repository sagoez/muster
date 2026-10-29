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
    adapter::holder::{
        ExecHolderLauncher, HOLDER_SUBCOMMAND, HolderDirs, HolderRecord, HolderRunner,
    },
    domain::{
        color_scheme::ColorScheme,
        port::{OutputSink, ProcessRunner},
        process::ProcessKind,
        pty::{ExitOutcome, ProcessOutput, PtySize, SpawnRequest},
        value::{ByteSize, Cols, CommandLine, ProcessName, Rows},
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
const SCRATCH_PREFIX: &str = "mx-";
/// Log cap of the test runners.
const LOG_CAP_MIB: u64 = 4;
/// Scrollback lines of the test runners' holders.
const SCROLLBACK_LINES: usize = 1000;
/// The child the holder runs.
const TICKER: &str = "while :; do echo tick; sleep 0.05; done";
/// What the ticker prints.
const TICK: &str = "tick";

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

/// Whether `output` is a chunk showing a tick.
fn is_tick(output: &ProcessOutput) -> bool {
    matches!(output, ProcessOutput::Chunk(bytes) if String::from_utf8_lossy(bytes).contains(TICK))
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
        let root = tempfile::Builder::new()
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
            .build();
        HolderRunner::new(
            Box::new(launcher),
            self.dirs.clone(),
            ByteSize::try_new(RawByteSize::mib(LOG_CAP_MIB)).unwrap(),
            SCROLLBACK_LINES,
            ColorScheme::Dark,
        )
    }

    fn request(&self) -> SpawnRequest {
        SpawnRequest::builder()
            .process(ProcessName::try_new("ticker").unwrap())
            .kind(ProcessKind::Command)
            .command(Some(CommandLine::try_new(TICKER).unwrap()))
            .project(Some(self.root.path().join("muster.yml")))
            .size(
                PtySize::builder()
                    .rows(Rows::new(24))
                    .cols(Cols::new(80))
                    .build(),
            )
            .build()
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        for record in HolderRecord::load_all(self.dirs.records()) {
            if record.child_pid() != process::id() {
                // SAFETY: `kill` takes plain integers and touches no memory.
                unsafe {
                    libc::kill(-(record.child_pid() as libc::pid_t), libc::SIGKILL);
                }
            }
        }
        for holder in holders_under(self.root.path()) {
            // SAFETY: `kill` takes plain integers and touches no memory.
            unsafe {
                libc::kill(holder as libc::pid_t, libc::SIGKILL);
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
        record.holder_pid(),
        process::id(),
        "the holder is its own process"
    );
    assert_eq!(holders_under(scratch.root.path()), vec![
        record.holder_pid()
    ]);
    drop(handle);
    drop(runner);

    let runner = scratch.runner();
    assert_eq!(runner.live_records(), vec![record.clone()]);
    let (sender, outputs) = unbounded();
    let mut adopted = runner
        .adopt(&record, Box::new(ChannelSink(sender)))
        .unwrap();
    assert_eq!(adopted.process_id(), Some(record.child_pid()));
    receive_until(&outputs, "the adoption snapshot", |output| {
        matches!(output, ProcessOutput::Chunk(_))
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
