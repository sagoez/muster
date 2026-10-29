use std::{
    env,
    ffi::CStr,
    fmt::Display,
    io::{Read, Write},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant},
};

use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, TryRecvError, bounded};
use portable_pty::{CommandBuilder, ExitStatus, PtySize as PortablePtySize, native_pty_system};
use rustix::process::Signal;
use signal_hook::low_level;

use super::environment::baseline_environment;
use crate::{
    adapter::signal_target::SignalTarget,
    constants::MUSTER_PROJECT_ENV,
    domain::{
        port::{OutputSink, ProcessHandle, ProcessInput, ProcessRunner},
        process::StopSignal,
        pty::{
            ExitCause, ExitOutcome, ProcessOutput, PtyError, PtySize, ScreenSource, SignalName,
            SpawnRequest,
        },
        value::Pid,
    },
};

/// Size of a single PTY read buffer, in bytes.
const READ_BUFFER_BYTES: usize = 4096;
/// POSIX shell that interprets each command line muster runs.
pub const SHELL_PROGRAM: &str = "/bin/sh";
/// POSIX flag that runs the following argument as a shell command.
pub const SHELL_COMMAND_FLAG: &str = "-c";
/// Environment variable naming the terminal type.
const TERM_VAR: &str = "TERM";
/// Terminal type advertised to children when the environment has none.
const DEFAULT_TERM: &str = "xterm-256color";
/// Environment variable tracking shell-nesting depth.
const SHELL_LEVEL_VAR: &str = "SHLVL";
/// Base shell level given to every pane. Muster is launched from the user's
/// shell, so it inherits a non-zero `SHLVL`; passing that through would start a
/// pane's shell one level deeper than a shell in a fresh terminal and break
/// setups keyed on `SHLVL == 1` (for example a login shell that only switches to
/// the user's preferred shell at the top level). Each pane is its own fresh
/// terminal, so it starts from the base level and the shell increments it to 1.
const BASE_SHELL_LEVEL: &str = "0";
/// Grace the waiter gives the reader to drain a child's final output before
/// reporting exit. Bounded so a lingering descendant never blocks the report.
const EXIT_DRAIN_GRACE: Duration = Duration::from_millis(200);
/// Poll interval used while a direct child has exited but its process group is
/// still completing descendant cleanup.
const PROCESS_GROUP_POLL_INTERVAL: Duration = Duration::from_millis(10);
/// Reported when a writer panicked mid-write, leaving the PTY input unusable.
const INPUT_POISONED: &str = "pty input writer is poisoned";

/// Spawns processes under a native PTY using `portable-pty`.
#[derive(Clone, Copy, Default)]
pub struct PortablePtyRunner;

impl ProcessRunner for PortablePtyRunner {
    fn spawn(
        &self,
        request: SpawnRequest,
        sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        let pair = native_pty_system()
            .openpty(to_portable_size(request.size()))
            .map_err(system_error)?;

        let mut command = CommandBuilder::new(SHELL_PROGRAM);
        command.arg(SHELL_COMMAND_FLAG);
        command.arg(request.command().as_ref());
        ensure_working_dir(&request)?;
        if let Some(dir) = request.working_dir() {
            command.cwd(dir);
        } else if let Ok(cwd) = env::current_dir() {
            command.cwd(cwd);
        }
        apply_environment(&mut command, &request);

        let mut child = pair.slave.spawn_command(command).map_err(system_error)?;
        let mut killer = child.clone_killer();
        let mut waiter_killer = child.clone_killer();
        let pid = child.process_id().and_then(|raw| Pid::try_new(raw).ok());
        drop(pair.slave);

        let io = pair
            .master
            .try_clone_reader()
            .and_then(|reader| pair.master.take_writer().map(|writer| (reader, writer)));
        let (mut reader, writer) = match io {
            Ok(io) => io,
            Err(error) => {
                let _ = killer.kill();
                return Err(system_error(error));
            },
        };

        let sink: Arc<dyn OutputSink> = Arc::from(sink);
        let (reader_done_tx, reader_done_rx) = bounded(1);
        let (grace_tx, grace_rx) = bounded(1);

        let reader_sink = Arc::clone(&sink);
        let reader_handle = thread::spawn(move || {
            let mut buffer = [0u8; READ_BUFFER_BYTES];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => reader_sink.send(ProcessOutput::Chunk(buffer[..read].to_vec())),
                }
            }
            let _ = reader_done_tx.send(());
        });
        thread::spawn(move || {
            let outcome = match child.wait() {
                Ok(status) => exit_outcome(&status),
                Err(_) => ExitOutcome::FAILED_UNKNOWN,
            };
            // A descendant can retain the PTY after the direct shell exits. A
            // graceful stop supplies its full cleanup window; ordinary exits use
            // only the short drain bound. The completion channel wakes this wait
            // immediately when cleanup finishes instead of delaying the exit event.
            let drain_grace = grace_rx.try_recv().unwrap_or(EXIT_DRAIN_GRACE);
            if wait_for_drain(&reader_done_rx, drain_grace, || {
                pid.is_some_and(|pid| SignalTarget::Group(pid).is_alive())
            }) == DrainStatus::TimedOut
            {
                let _ = terminate_group(pid, &mut waiter_killer);
                let _ = reader_done_rx.recv();
            }
            // Joining before publishing exit sequences the exit strictly after
            // every chunk sent, so channel backpressure cannot make output stale.
            let _ = reader_handle.join();
            sink.send(ProcessOutput::Exited(outcome));
        });

        Ok(Box::new(PtyProcessHandle {
            master: pair.master,
            input: PtyInput(Arc::new(Mutex::new(writer))),
            killer,
            grace_tx,
            pid,
        }))
    }
}

/// Gives `command` exactly the environment `request`'s child runs with: the
/// scrubbed baseline, then the environment of the command that launched it,
/// then its own stored variables, then muster's: a terminal type when none
/// was given, the base shell level and the project root.
fn apply_environment(command: &mut CommandBuilder, request: &SpawnRequest) {
    command.env_clear();
    let layers = [
        baseline_environment(),
        request.launch_environment().clone(),
        request.environment().clone(),
    ];
    for (name, value) in layers.into_iter().flatten() {
        command.env(name, value);
    }
    if command.get_env(TERM_VAR).is_none() {
        command.env(TERM_VAR, DEFAULT_TERM);
    }
    command.env(SHELL_LEVEL_VAR, BASE_SHELL_LEVEL);
    if let Some(project) = request.project() {
        command.env(MUSTER_PROJECT_ENV, project);
    }
}

/// Refuses a request whose configured working directory is missing or not a
/// directory, before anything is spawned.
///
/// # Errors
/// Returns [`PtyError::InvalidWorkingDir`] naming the bad directory.
pub fn ensure_working_dir(request: &SpawnRequest) -> Result<(), PtyError> {
    match request.working_dir() {
        Some(dir) if !dir.is_dir() => Err(PtyError::InvalidWorkingDir(dir.clone())),
        _ => Ok(()),
    }
}

/// Outcome of waiting for both PTY output and descendant cleanup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DrainStatus {
    Complete,
    TimedOut,
}

/// Waits until the PTY reader finishes and the process group is empty, bounded
/// by `grace`. The liveness callback keeps the timing logic directly testable.
fn wait_for_drain<F>(
    reader_done_rx: &Receiver<()>,
    grace: Duration,
    mut group_is_alive: F,
) -> DrainStatus
where
    F: FnMut() -> bool,
{
    let started = Instant::now();
    let mut reader_done = match reader_done_rx.try_recv() {
        Ok(()) | Err(TryRecvError::Disconnected) => true,
        Err(TryRecvError::Empty) => false,
    };
    loop {
        if reader_done && !group_is_alive() {
            return DrainStatus::Complete;
        }

        let remaining = grace.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            return DrainStatus::TimedOut;
        }
        let poll = remaining.min(PROCESS_GROUP_POLL_INTERVAL);
        if reader_done {
            thread::sleep(poll);
            continue;
        }
        match reader_done_rx.recv_timeout(poll) {
            Ok(()) | Err(RecvTimeoutError::Disconnected) => reader_done = true,
            Err(RecvTimeoutError::Timeout) => {},
        }
    }
}

/// The PTY's input writer, shared so another thread can write while the
/// handle's owner keeps controlling the process.
#[derive(Clone)]
struct PtyInput(Arc<Mutex<Box<dyn Write + Send>>>);

impl ProcessInput for PtyInput {
    /// Writes and flushes `bytes` to the PTY.
    ///
    /// # Errors
    /// Returns a `PtyError` if the write fails or a writer panicked mid-write.
    fn write_input(&mut self, bytes: &[u8]) -> Result<(), PtyError> {
        let mut writer = self
            .0
            .lock()
            .map_err(|_| PtyError::System(INPUT_POISONED.to_string()))?;
        writer.write_all(bytes)?;
        writer.flush()?;
        Ok(())
    }
}

/// Live handle to a PTY-backed process.
struct PtyProcessHandle {
    master: Box<dyn portable_pty::MasterPty + Send>,
    input: PtyInput,
    killer: Box<dyn portable_pty::ChildKiller + Send + Sync>,
    grace_tx: Sender<Duration>,
    pid: Option<Pid>,
}

impl ProcessHandle for PtyProcessHandle {
    fn process_id(&self) -> Option<Pid> {
        self.pid
    }

    fn input(&self) -> Option<Box<dyn ProcessInput>> {
        Some(Box::new(self.input.clone()))
    }

    /// A bare PTY has nothing but its reader's rendering of the output.
    fn screen_source(&self) -> ScreenSource {
        ScreenSource::Kept
    }

    /// Kills the process group; nothing else holds the process.
    fn abandon(mut self: Box<Self>) {
        let _ = self.kill();
    }

    fn write_input(&mut self, bytes: &[u8]) -> Result<(), PtyError> {
        self.input.write_input(bytes)
    }

    fn resize(&mut self, size: PtySize) -> Result<(), PtyError> {
        self.master
            .resize(to_portable_size(size))
            .map_err(system_error)
    }

    fn pause(&mut self) -> Result<(), PtyError> {
        signal_group(self.group()?, Signal::STOP)
    }

    fn resume(&mut self) -> Result<(), PtyError> {
        signal_group(self.group()?, Signal::CONT)
    }

    fn terminate(&mut self, signal: StopSignal, grace: Duration) -> Result<(), PtyError> {
        // Publish the grace before signalling: if the signal makes the direct shell
        // exit immediately, its waiter still observes the intended deadline.
        let _ = self.grace_tx.try_send(grace);
        // A paused command cannot handle a shutdown signal until it resumes.
        let group = self.group()?;
        let _ = signal_group(group, Signal::CONT);
        signal_group(group, unix_signal(signal))
    }

    fn kill(&mut self) -> Result<(), PtyError> {
        terminate_group(self.pid, &mut self.killer)
    }
}

impl PtyProcessHandle {
    /// The process group the child leads, which every signal reaches.
    ///
    /// # Errors
    /// Returns [`PtyError::Unsupported`] when the platform gave no pid.
    fn group(&self) -> Result<Pid, PtyError> {
        signallable(self.pid)
    }
}

/// Reported when a child has no pid to signal.
const NO_PID: &str = "no pid to signal";
/// Highest signal number tried when recovering a signal from its description.
const MAX_SIGNAL: libc::c_int = 64;
/// How portable-pty describes a signal `strsignal` has no text for.
const UNDESCRIBED_SIGNAL_PREFIX: &str = "Signal ";
/// Name of a signal with no POSIX name, before its number.
const UNNAMED_SIGNAL_PREFIX: &str = "signal-";

/// `pid`, when there is one to signal.
///
/// # Errors
/// Returns [`PtyError::Unsupported`] when there is none.
fn signallable(pid: Option<Pid>) -> Result<Pid, PtyError> {
    pid.ok_or_else(|| PtyError::Unsupported(NO_PID.to_string()))
}

/// Maps the domain shutdown signal to its Unix signal.
pub(crate) fn unix_signal(signal: StopSignal) -> Signal {
    match signal {
        StopSignal::Terminate => Signal::TERM,
        StopSignal::Interrupt => Signal::INT,
    }
}

/// How a child that ended with `status` exited: success, or the status or
/// signal that ended it.
fn exit_outcome(status: &ExitStatus) -> ExitOutcome {
    match status.signal() {
        Some(described) => match signal_name(described) {
            Some(name) => ExitOutcome::Failed(ExitCause::Signal(name)),
            None => ExitOutcome::FAILED_UNKNOWN,
        },
        None if status.success() => ExitOutcome::Succeeded,
        None => ExitOutcome::Failed(ExitCause::Status(status.exit_code())),
    }
}

/// The name of the signal portable-pty described as `described`, or `None`
/// when no signal reads that way. portable-pty keeps only `strsignal`'s text,
/// which depends on the locale, so the number is recovered by describing each
/// signal the same way in this process and the name derived from the number.
fn signal_name(described: &str) -> Option<SignalName> {
    let number = described
        .strip_prefix(UNDESCRIBED_SIGNAL_PREFIX)
        .and_then(|number| number.parse::<libc::c_int>().ok())
        .or_else(|| {
            (1..=MAX_SIGNAL).find(|&number| strsignal_text(number).as_deref() == Some(described))
        })?;
    let token = match low_level::signal_name(number) {
        Some(posix) => posix.to_string(),
        None => format!("{UNNAMED_SIGNAL_PREFIX}{number}"),
    };
    SignalName::try_new(token).ok()
}

/// How `strsignal` describes the signal `number` in this process's locale.
fn strsignal_text(number: libc::c_int) -> Option<String> {
    // SAFETY: `strsignal` returns null or a NUL-terminated string that stays
    // valid until the next call on this thread; it is copied out at once.
    unsafe {
        let text = libc::strsignal(number);
        (!text.is_null()).then(|| CStr::from_ptr(text).to_string_lossy().into_owned())
    }
}

/// Converts a domain [`PtySize`] into the crate's PTY size (pixel size unused).
fn to_portable_size(size: PtySize) -> PortablePtySize {
    PortablePtySize {
        rows: size.rows().into_inner(),
        cols: size.cols().into_inner(),
        pixel_width: 0,
        pixel_height: 0,
    }
}

/// Maps a `portable-pty` error (any `Display` error) into a [`PtyError::System`].
fn system_error<E: Display>(error: E) -> PtyError {
    PtyError::System(error.to_string())
}

/// Sends `signal` to the whole process group `pid` leads, so it reaches
/// backgrounded descendants and not just the direct child.
///
/// # Errors
/// Returns the OS error when the signal is refused, such as when the group
/// has already exited or may not be signalled.
pub(crate) fn signal_group(pid: Pid, signal: Signal) -> Result<(), PtyError> {
    Ok(SignalTarget::Group(pid).send(signal)?)
}

/// Terminates a spawned process: a whole-group SIGKILL, with the portable child
/// killer as the fallback. Returns success when either mechanism delivers the
/// request.
///
/// # Errors
/// Returns a `PtyError` when neither termination mechanism succeeds.
fn terminate_group(
    pid: Option<Pid>,
    killer: &mut Box<dyn portable_pty::ChildKiller + Send + Sync>,
) -> Result<(), PtyError> {
    let group_result = signallable(pid).and_then(|pid| signal_group(pid, Signal::KILL));
    let child_result = killer.kill().map_err(system_error);
    match (group_result, child_result) {
        (Ok(()), _) | (_, Ok(())) => Ok(()),
        (Err(error), Err(_)) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        path::PathBuf,
        time::{Duration, Instant},
    };

    use bytesize::ByteSize as RawByteSize;

    use super::*;
    use crate::domain::{
        color_scheme::ColorScheme,
        process::StopPolicy,
        value::{ByteSize, CommandLine, ProcessName, StopGrace},
    };

    /// Name given to every test process.
    const TEST_PROCESS: &str = "test";
    /// Scrollback lines every test request asks for.
    const TEST_SCROLLBACK: usize = 100;
    /// Maximum time to wait for a test process's events.
    const OUTPUT_TIMEOUT: Duration = Duration::from_secs(5);
    /// Grace used by termination tests, long enough for delayed descendant
    /// cleanup while keeping the suite quick.
    const TEST_STOP_GRACE: Duration = Duration::from_secs(3);
    /// Minimum time the macOS waiter must preserve for descendant cleanup after
    /// the PTY reader reports the direct shell's exit.
    #[cfg(target_os = "macos")]
    const DESCENDANT_CLEANUP_MIN_WAIT: Duration = Duration::from_millis(750);
    /// Shell fixture whose direct wrapper exits on TERM while a signal-resistant
    /// descendant takes longer than the ordinary PTY drain bound to finish. The
    /// final sleep models cleanup that remains active after its last log line.
    const DESCENDANT_CLEANUP_COMMAND: &str = r#"sh -c 'trap "" TERM HUP; printf ready; sleep 1; printf descendant-clean; sleep 1' & trap 'exit 0' TERM; wait"#;

    struct ChannelSink(crossbeam_channel::Sender<ProcessOutput>);

    impl OutputSink for ChannelSink {
        fn send(&self, output: ProcessOutput) {
            let _ = self.0.send(output);
        }
    }

    /// A request to run `command`, with every launch setting explicit.
    fn request(command: &str) -> SpawnRequest {
        request_in(command, None)
    }

    /// As [`request`], launched in `working_dir` when given.
    fn request_in(command: &str, working_dir: Option<PathBuf>) -> SpawnRequest {
        SpawnRequest::builder()
            .process(ProcessName::try_new(TEST_PROCESS).unwrap())
            .command(CommandLine::try_new(command).unwrap())
            .working_dir(working_dir)
            .size(PtySize::try_new(24, 80).unwrap())
            .scrollback(TEST_SCROLLBACK)
            .log_max_size(ByteSize::try_new(RawByteSize::mib(1)).unwrap())
            .color_scheme(ColorScheme::Dark)
            .stop(
                StopPolicy::builder()
                    .signal(StopSignal::Terminate)
                    .grace_period(StopGrace::try_new(TEST_STOP_GRACE).unwrap())
                    .build(),
            )
            .build()
    }

    /// Everything `request` prints before it exits.
    fn output_of(request: SpawnRequest) -> String {
        let (tx, rx) = crossbeam_channel::unbounded();
        let _handle = PortablePtyRunner
            .spawn(request, Box::new(ChannelSink(tx)))
            .unwrap();
        let mut bytes = Vec::new();
        while let Ok(output) = rx.recv_timeout(OUTPUT_TIMEOUT) {
            match output {
                ProcessOutput::Chunk(chunk) => bytes.extend(chunk),
                ProcessOutput::Exited(_) => break,
                ProcessOutput::Unreadable(_)
                | ProcessOutput::Warning(_)
                | ProcessOutput::Attached(_)
                | ProcessOutput::Placed { .. }
                | ProcessOutput::Repaint { .. } => {},
            }
        }
        String::from_utf8_lossy(&bytes).into_owned()
    }

    /// The child's environment is the baseline, then the launching command's,
    /// then its own, then muster's project root: nothing else this process
    /// inherited reaches it, and each layer overrides the one before.
    #[test]
    fn a_child_runs_with_the_layered_environment_and_nothing_else() {
        let project = PathBuf::from("/srv/shop");
        let request = SpawnRequest::builder()
            .process(ProcessName::try_new(TEST_PROCESS).unwrap())
            .command(
                CommandLine::try_new(
                    "printf 'from=%s own=%s both=%s project=%s home=%s cargo=%s' \
                     \"$FROM_CLI\" \"$OWN\" \"$BOTH\" \"$MUSTER_PROJECT\" \"$HOME\" \
                     \"${CARGO_PKG_NAME:-unset}\"",
                )
                .unwrap(),
            )
            .project(Some(project))
            .launch_environment(
                [
                    ("FROM_CLI".into(), "cli".into()),
                    ("BOTH".into(), "cli".into()),
                ]
                .into(),
            )
            .environment([("OWN".into(), "own".into()), ("BOTH".into(), "own".into())].into())
            .size(PtySize::try_new(24, 80).unwrap())
            .scrollback(TEST_SCROLLBACK)
            .log_max_size(ByteSize::try_new(RawByteSize::mib(1)).unwrap())
            .color_scheme(ColorScheme::Dark)
            .stop(request("true").stop())
            .build();

        let output = output_of(request);

        let home = env::var("HOME").unwrap_or_default();
        assert!(
            output.contains(&format!(
                "from=cli own=own both=own project=/srv/shop home={home} cargo=unset"
            )),
            "{output:?}"
        );
    }

    /// Ensures an early PTY EOF does not bypass a still-live process group.
    #[test]
    fn drain_waits_for_the_process_group_after_the_reader_finishes() {
        let (tx, rx) = bounded(1);
        tx.send(()).unwrap();
        drop(tx);
        let polls = Cell::new(0);

        let status = wait_for_drain(&rx, OUTPUT_TIMEOUT, || {
            let next = polls.get() + 1;
            polls.set(next);
            next < 3
        });

        assert_eq!(status, DrainStatus::Complete);
        assert_eq!(polls.get(), 3);
    }

    #[test]
    fn streams_output_then_reports_success() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let _handle = PortablePtyRunner
            .spawn(request("printf hello"), Box::new(ChannelSink(tx)))
            .unwrap();

        let mut bytes = Vec::new();
        let mut outcome = None;
        while let Ok(output) = rx.recv_timeout(OUTPUT_TIMEOUT) {
            match output {
                ProcessOutput::Chunk(chunk) => bytes.extend_from_slice(&chunk),
                ProcessOutput::Exited(exit) => outcome = Some(exit),
                other @ (ProcessOutput::Unreadable(_)
                | ProcessOutput::Warning(_)
                | ProcessOutput::Attached(_)
                | ProcessOutput::Placed { .. }
                | ProcessOutput::Repaint { .. }) => panic!("unexpected output: {other:?}"),
            }
        }

        assert!(String::from_utf8_lossy(&bytes).contains("hello"));
        assert_eq!(outcome, Some(ExitOutcome::Succeeded));
    }

    #[test]
    fn final_output_is_delivered_before_exit() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let _handle = PortablePtyRunner
            .spawn(request("printf hello; exit 1"), Box::new(ChannelSink(tx)))
            .unwrap();

        let mut events = Vec::new();
        while let Ok(output) = rx.recv_timeout(OUTPUT_TIMEOUT) {
            events.push(output);
        }

        let exit_pos = events
            .iter()
            .position(|event| matches!(event, ProcessOutput::Exited(_)))
            .expect("an exit is reported");
        assert_eq!(exit_pos, events.len() - 1, "exit must be the final event");
        let output: Vec<u8> = events[..exit_pos]
            .iter()
            .flat_map(|event| match event {
                ProcessOutput::Chunk(chunk) => chunk.clone(),
                ProcessOutput::Exited(_)
                | ProcessOutput::Unreadable(_)
                | ProcessOutput::Warning(_)
                | ProcessOutput::Attached(_)
                | ProcessOutput::Placed { .. }
                | ProcessOutput::Repaint { .. } => Vec::new(),
            })
            .collect();
        assert!(String::from_utf8_lossy(&output).contains("hello"));
    }

    #[test]
    fn graceful_termination_drains_shutdown_output() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut handle = PortablePtyRunner
            .spawn(
                request(
                    "trap 'printf shutdown; exit 0' TERM; printf ready; while :; do sleep 1; done",
                ),
                Box::new(ChannelSink(tx)),
            )
            .unwrap();

        let mut bytes = Vec::new();
        while !String::from_utf8_lossy(&bytes).contains("ready") {
            match rx.recv_timeout(OUTPUT_TIMEOUT).unwrap() {
                ProcessOutput::Chunk(chunk) => bytes.extend(chunk),
                ProcessOutput::Exited(_) => panic!("command exited before termination"),
                other @ (ProcessOutput::Unreadable(_)
                | ProcessOutput::Warning(_)
                | ProcessOutput::Attached(_)
                | ProcessOutput::Placed { .. }
                | ProcessOutput::Repaint { .. }) => panic!("unexpected output: {other:?}"),
            }
        }
        handle
            .terminate(StopSignal::Terminate, TEST_STOP_GRACE)
            .unwrap();
        while let Ok(output) = rx.recv_timeout(OUTPUT_TIMEOUT) {
            match output {
                ProcessOutput::Chunk(chunk) => bytes.extend(chunk),
                ProcessOutput::Exited(_) => break,
                other @ (ProcessOutput::Unreadable(_)
                | ProcessOutput::Warning(_)
                | ProcessOutput::Attached(_)
                | ProcessOutput::Placed { .. }
                | ProcessOutput::Repaint { .. }) => panic!("unexpected output: {other:?}"),
            }
        }

        assert!(String::from_utf8_lossy(&bytes).contains("shutdown"));
    }

    /// A pane starts from the base nesting level muster passes (`SHLVL=0`), never
    /// muster's own inherited, elevated level. Its shell therefore reports at most
    /// one level: shells that count a non-interactive `sh -c` (bash) report 1,
    /// those that do not (dash, the usual `/bin/sh` on CI) report 0. Either way a
    /// setup that only switches shells at the top level behaves inside a pane as in
    /// a bare terminal.
    #[test]
    fn a_pane_shell_starts_at_the_base_nesting_level() {
        let (tx, rx) = crossbeam_channel::unbounded();
        PortablePtyRunner
            .spawn(
                request("printf 'shlvl=%s' \"$SHLVL\""),
                Box::new(ChannelSink(tx)),
            )
            .unwrap();

        let mut bytes = Vec::new();
        while let ProcessOutput::Chunk(chunk) = rx.recv_timeout(OUTPUT_TIMEOUT).unwrap() {
            bytes.extend(chunk);
        }

        let output = String::from_utf8_lossy(&bytes);
        let level: u32 = output
            .rsplit("shlvl=")
            .next()
            .and_then(|value| value.trim().parse().ok())
            .unwrap_or_else(|| panic!("no shell level in pane output: {output:?}"));
        assert!(level <= 1, "pane shell nested at level {level}: {output:?}");
    }

    #[test]
    fn graceful_termination_preserves_descendant_cleanup_after_shell_exit() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let mut handle = PortablePtyRunner
            .spawn(
                request(DESCENDANT_CLEANUP_COMMAND),
                Box::new(ChannelSink(tx)),
            )
            .unwrap();

        let mut bytes = Vec::new();
        while !String::from_utf8_lossy(&bytes).contains("ready") {
            match rx.recv_timeout(OUTPUT_TIMEOUT).unwrap() {
                ProcessOutput::Chunk(chunk) => bytes.extend(chunk),
                ProcessOutput::Exited(_) => panic!("command exited before termination"),
                other @ (ProcessOutput::Unreadable(_)
                | ProcessOutput::Warning(_)
                | ProcessOutput::Attached(_)
                | ProcessOutput::Placed { .. }
                | ProcessOutput::Repaint { .. }) => panic!("unexpected output: {other:?}"),
            }
        }
        #[cfg(target_os = "macos")]
        let termination_started = Instant::now();
        handle
            .terminate(StopSignal::Terminate, TEST_STOP_GRACE)
            .unwrap();
        let mut exited = false;
        while let Ok(output) = rx.recv_timeout(OUTPUT_TIMEOUT) {
            match output {
                ProcessOutput::Chunk(chunk) => bytes.extend(chunk),
                ProcessOutput::Exited(_) => {
                    exited = true;
                    break;
                },
                other @ (ProcessOutput::Unreadable(_)
                | ProcessOutput::Warning(_)
                | ProcessOutput::Attached(_)
                | ProcessOutput::Placed { .. }
                | ProcessOutput::Repaint { .. }) => panic!("unexpected output: {other:?}"),
            }
        }

        assert!(exited, "the runner must report the completed cleanup");
        #[cfg(target_os = "macos")]
        assert!(
            termination_started.elapsed() >= DESCENDANT_CLEANUP_MIN_WAIT,
            "an early macOS PTY EOF must not bypass descendant cleanup"
        );
        #[cfg(not(target_os = "macos"))]
        assert!(String::from_utf8_lossy(&bytes).contains("descendant-clean"));
    }

    #[test]
    fn a_slow_drain_delivers_all_output_before_exit() {
        // A bounded channel drained slowly makes the reader outlast the grace
        // window; a fixed timeout would truncate, progress-based waiting must not.
        const OUTPUT_LEN: usize = 200_000;
        let (tx, rx) = crossbeam_channel::bounded(1);
        let _handle = PortablePtyRunner
            .spawn(
                request(r"head -c 200000 /dev/zero | tr '\0' x"),
                Box::new(ChannelSink(tx)),
            )
            .unwrap();

        let mut events = Vec::new();
        while let Ok(output) = rx.recv_timeout(OUTPUT_TIMEOUT) {
            events.push(output);
            thread::sleep(Duration::from_millis(5));
        }

        let exit_pos = events
            .iter()
            .position(|event| matches!(event, ProcessOutput::Exited(_)))
            .expect("an exit is reported");
        assert_eq!(exit_pos, events.len() - 1, "exit must be the final event");
        let total: usize = events[..exit_pos]
            .iter()
            .map(|event| match event {
                ProcessOutput::Chunk(chunk) => chunk.len(),
                ProcessOutput::Exited(_)
                | ProcessOutput::Unreadable(_)
                | ProcessOutput::Warning(_)
                | ProcessOutput::Attached(_)
                | ProcessOutput::Placed { .. }
                | ProcessOutput::Repaint { .. } => 0,
            })
            .sum();
        assert_eq!(
            total, OUTPUT_LEN,
            "all output delivered, not truncated by the exit"
        );
    }

    #[test]
    fn reports_failure_for_nonzero_exit() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let _handle = PortablePtyRunner
            .spawn(request("exit 3"), Box::new(ChannelSink(tx)))
            .unwrap();

        let mut outcome = None;
        while let Ok(output) = rx.recv_timeout(OUTPUT_TIMEOUT) {
            if let ProcessOutput::Exited(exit) = output {
                outcome = Some(exit);
            }
        }

        assert_eq!(outcome, Some(ExitOutcome::Failed(ExitCause::Status(3))));
    }

    /// A child ended by a signal reports the signal, not a status.
    #[test]
    fn reports_the_signal_that_ended_a_child() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let _handle = PortablePtyRunner
            .spawn(request("kill -KILL $$"), Box::new(ChannelSink(tx)))
            .unwrap();

        let mut outcome = None;
        while let Ok(output) = rx.recv_timeout(OUTPUT_TIMEOUT) {
            if let ProcessOutput::Exited(exit) = output {
                outcome = Some(exit);
            }
        }

        assert_eq!(
            outcome,
            Some(ExitOutcome::Failed(ExitCause::Signal(
                SignalName::try_new("sigkill").unwrap()
            )))
        );
    }

    /// A signal is named from its number, whichever way the OS words it.
    #[test]
    fn a_signal_is_named_from_its_number() {
        assert_eq!(
            signal_name(&strsignal_text(libc::SIGTERM).unwrap()),
            Some(SignalName::try_new("sigterm").unwrap())
        );
        assert_eq!(
            signal_name("Signal 63"),
            Some(SignalName::try_new("signal-63").unwrap()),
            "a signal with no POSIX name is named by its number"
        );
        assert_eq!(signal_name("no such signal"), None);
    }

    #[test]
    fn invalid_working_directory_is_rejected() {
        let (tx, _rx) = crossbeam_channel::unbounded();
        let request = request_in("true", Some(PathBuf::from("/no/such/muster/dir")));

        let result = PortablePtyRunner.spawn(request, Box::new(ChannelSink(tx)));
        assert!(matches!(result, Err(PtyError::InvalidWorkingDir(_))));
    }

    #[test]
    fn inherits_the_current_directory_when_no_working_dir_is_set() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let _handle = PortablePtyRunner
            .spawn(request("pwd -P"), Box::new(ChannelSink(tx)))
            .unwrap();

        let mut bytes = Vec::new();
        while let Ok(output) = rx.recv_timeout(OUTPUT_TIMEOUT) {
            if let ProcessOutput::Chunk(chunk) = output {
                bytes.extend_from_slice(&chunk);
            }
        }

        let expected = std::env::current_dir().unwrap();
        assert!(String::from_utf8_lossy(&bytes).contains(expected.to_str().unwrap()));
    }

    #[test]
    fn exit_is_observed_before_a_backgrounded_descendant_closes_the_pty() {
        let (tx, rx) = crossbeam_channel::unbounded();
        let start = Instant::now();
        let _handle = PortablePtyRunner
            .spawn(request("sleep 5 &"), Box::new(ChannelSink(tx)))
            .unwrap();

        loop {
            match rx.recv_timeout(OUTPUT_TIMEOUT) {
                Ok(ProcessOutput::Exited(_)) => break,
                Ok(_) => {},
                Err(_) => break,
            }
        }

        assert!(start.elapsed() < Duration::from_secs(2));
    }
}
