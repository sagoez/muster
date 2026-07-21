use std::{io::Read, sync::Arc, thread, time::Duration};

use crossbeam_channel::{Sender, bounded};
use portable_pty::{CommandBuilder, PtySize as PortablePtySize, native_pty_system};

use crate::{
    constants::MUSTER_PROJECT_ENV,
    domain::{
        port::{OutputSink, ProcessHandle, ProcessRunner},
        process::StopSignal,
        pty::{ExitOutcome, ProcessOutput, PtyError, PtySize, SpawnRequest},
    },
};

/// Size of a single PTY read buffer, in bytes.
const READ_BUFFER_BYTES: usize = 4096;
/// Shell used to interpret each process's command line.
const SHELL_PROGRAM: &str = "/bin/sh";
/// Flag that runs the following argument as a shell command.
const SHELL_COMMAND_FLAG: &str = "-c";
/// Environment variable naming the terminal type.
const TERM_VAR: &str = "TERM";
/// Terminal type advertised to children when the environment has none.
const DEFAULT_TERM: &str = "xterm-256color";
/// Grace the waiter gives the reader to drain a child's final output before
/// reporting exit. Bounded so a lingering descendant never blocks the report.
const EXIT_DRAIN_GRACE: Duration = Duration::from_millis(200);
/// Reported when suspend/resume is requested on a platform without job-control
/// signals.
#[cfg(not(unix))]
const SUSPEND_UNSUPPORTED: &str = "suspend and resume are only supported on Unix";

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

        let mut command = match request.command() {
            Some(command_line) => {
                let mut builder = CommandBuilder::new(SHELL_PROGRAM);
                builder.arg(SHELL_COMMAND_FLAG);
                builder.arg(command_line.as_ref());
                builder
            },
            None => CommandBuilder::new_default_prog(),
        };
        if let Some(dir) = request.working_dir() {
            if !dir.is_dir() {
                return Err(PtyError::InvalidWorkingDir(dir.clone()));
            }
            command.cwd(dir);
        } else if let Ok(cwd) = std::env::current_dir() {
            command.cwd(cwd);
        }
        if std::env::var_os(TERM_VAR).is_none() {
            command.env(TERM_VAR, DEFAULT_TERM);
        }
        if let Some(project) = request.project() {
            command.env(MUSTER_PROJECT_ENV, project);
        }

        let mut child = pair.slave.spawn_command(command).map_err(system_error)?;
        let mut killer = child.clone_killer();
        let mut waiter_killer = child.clone_killer();
        let pid = child.process_id();
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
                Ok(status) if status.success() => ExitOutcome::Succeeded,
                _ => ExitOutcome::Failed,
            };
            // A descendant can retain the PTY after the direct shell exits. A
            // graceful stop supplies its full cleanup window; ordinary exits use
            // only the short drain bound. The completion channel wakes this wait
            // immediately when cleanup finishes instead of delaying the exit event.
            let drain_grace = grace_rx.try_recv().unwrap_or(EXIT_DRAIN_GRACE);
            if reader_done_rx.recv_timeout(drain_grace).is_err() {
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
            writer,
            killer,
            grace_tx,
            pid,
        }))
    }
}

/// Live handle to a PTY-backed process.
struct PtyProcessHandle {
    master: Box<dyn portable_pty::MasterPty + Send>,
    writer: Box<dyn std::io::Write + Send>,
    killer: Box<dyn portable_pty::ChildKiller + Send + Sync>,
    grace_tx: Sender<Duration>,
    pid: Option<u32>,
}

impl ProcessHandle for PtyProcessHandle {
    fn write_input(&mut self, bytes: &[u8]) -> Result<(), PtyError> {
        self.writer.write_all(bytes)?;
        self.writer.flush()?;
        Ok(())
    }

    fn resize(&mut self, size: PtySize) -> Result<(), PtyError> {
        self.master
            .resize(to_portable_size(size))
            .map_err(system_error)
    }

    fn pause(&mut self) -> Result<(), PtyError> {
        #[cfg(unix)]
        {
            signal_group(self.pid, libc::SIGSTOP)
        }
        #[cfg(not(unix))]
        {
            Err(PtyError::Unsupported(SUSPEND_UNSUPPORTED.to_string()))
        }
    }

    fn resume(&mut self) -> Result<(), PtyError> {
        #[cfg(unix)]
        {
            signal_group(self.pid, libc::SIGCONT)
        }
        #[cfg(not(unix))]
        {
            Err(PtyError::Unsupported(SUSPEND_UNSUPPORTED.to_string()))
        }
    }

    fn terminate(&mut self, signal: StopSignal, grace: Duration) -> Result<(), PtyError> {
        // Publish the grace before signalling: if the signal makes the direct shell
        // exit immediately, its waiter still observes the intended deadline.
        let _ = self.grace_tx.try_send(grace);
        #[cfg(unix)]
        {
            // A paused command cannot handle a shutdown signal until it resumes.
            let _ = signal_group(self.pid, libc::SIGCONT);
            signal_group(self.pid, unix_signal(signal))
        }
        #[cfg(not(unix))]
        {
            let _ = signal;
            self.killer.kill().map_err(system_error)
        }
    }

    fn kill(&mut self) -> Result<(), PtyError> {
        terminate_group(self.pid, &mut self.killer)
    }
}

/// Maps the domain shutdown signal to its Unix signal number.
#[cfg(unix)]
fn unix_signal(signal: StopSignal) -> libc::c_int {
    match signal {
        StopSignal::Terminate => libc::SIGTERM,
        StopSignal::Interrupt => libc::SIGINT,
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
fn system_error<E: std::fmt::Display>(error: E) -> PtyError {
    PtyError::System(error.to_string())
}

/// Sends `signal` to a process's whole group (negative pid), so it reaches
/// backgrounded descendants and not just the direct child. Unix only.
///
/// # Errors
/// Returns `Unsupported` when there is no pid to target, or the OS error when
/// `kill` reports failure (e.g. the process has already exited, or `EPERM`).
#[cfg(unix)]
fn signal_group(pid: Option<u32>, signal: libc::c_int) -> Result<(), PtyError> {
    let pid = pid.ok_or_else(|| PtyError::Unsupported("no pid to signal".to_string()))?;
    if unsafe { libc::kill(-(pid as libc::pid_t), signal) } == -1 {
        return Err(PtyError::Io(std::io::Error::last_os_error()));
    }
    Ok(())
}

/// Terminates a spawned process: a whole-group SIGKILL on Unix, with the
/// portable child killer as the fallback and the non-Unix path. Returns success
/// when either mechanism delivers the request.
///
/// # Errors
/// Returns a `PtyError` when neither termination mechanism succeeds.
fn terminate_group(
    pid: Option<u32>,
    killer: &mut Box<dyn portable_pty::ChildKiller + Send + Sync>,
) -> Result<(), PtyError> {
    #[cfg(unix)]
    {
        let group_result = signal_group(pid, libc::SIGKILL);
        let child_result = killer.kill().map_err(system_error);
        match (group_result, child_result) {
            (Ok(()), _) | (_, Ok(())) => Ok(()),
            (Err(error), Err(_)) => Err(error),
        }
    }
    #[cfg(not(unix))]
    {
        let _ = pid;
        killer.kill().map_err(system_error)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        time::{Duration, Instant},
    };

    use super::*;
    use crate::domain::value::{Cols, CommandLine, Rows};

    /// Maximum time to wait for a test process's events.
    const OUTPUT_TIMEOUT: Duration = Duration::from_secs(5);
    /// Grace used by termination tests, long enough for delayed descendant
    /// cleanup while keeping the suite quick.
    const TEST_STOP_GRACE: Duration = Duration::from_secs(3);
    /// Shell fixture whose direct wrapper exits on TERM while a signal-resistant
    /// descendant takes longer than the ordinary PTY drain bound to finish. The
    /// final sleep keeps the slave open while macOS delivers the cleanup marker.
    const DESCENDANT_CLEANUP_COMMAND: &str = r#"sh -c 'trap "" TERM HUP; printf ready; sleep 1; printf descendant-clean; sleep 1' & trap 'exit 0' TERM; wait"#;

    struct ChannelSink(crossbeam_channel::Sender<ProcessOutput>);

    impl OutputSink for ChannelSink {
        fn send(&self, output: ProcessOutput) {
            let _ = self.0.send(output);
        }
    }

    fn request(command: &str) -> SpawnRequest {
        SpawnRequest::builder()
            .command(Some(CommandLine::try_new(command).unwrap()))
            .size(
                PtySize::builder()
                    .rows(Rows::new(24))
                    .cols(Cols::new(80))
                    .build(),
            )
            .build()
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
                ProcessOutput::Exited(_) => Vec::new(),
            })
            .collect();
        assert!(String::from_utf8_lossy(&output).contains("hello"));
    }

    #[cfg(unix)]
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
            }
        }
        handle
            .terminate(StopSignal::Terminate, TEST_STOP_GRACE)
            .unwrap();
        while let Ok(output) = rx.recv_timeout(OUTPUT_TIMEOUT) {
            match output {
                ProcessOutput::Chunk(chunk) => bytes.extend(chunk),
                ProcessOutput::Exited(_) => break,
            }
        }

        assert!(String::from_utf8_lossy(&bytes).contains("shutdown"));
    }

    #[cfg(unix)]
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
            }
        }
        handle
            .terminate(StopSignal::Terminate, TEST_STOP_GRACE)
            .unwrap();
        while let Ok(output) = rx.recv_timeout(OUTPUT_TIMEOUT) {
            match output {
                ProcessOutput::Chunk(chunk) => bytes.extend(chunk),
                ProcessOutput::Exited(_) => break,
            }
        }

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
                ProcessOutput::Exited(_) => 0,
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

        assert_eq!(outcome, Some(ExitOutcome::Failed));
    }

    #[test]
    fn invalid_working_directory_is_rejected() {
        let (tx, _rx) = crossbeam_channel::unbounded();
        let request = SpawnRequest::builder()
            .command(Some(CommandLine::try_new("true").unwrap()))
            .working_dir(Some(PathBuf::from("/no/such/muster/dir")))
            .size(
                PtySize::builder()
                    .rows(Rows::new(24))
                    .cols(Cols::new(80))
                    .build(),
            )
            .build();

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
