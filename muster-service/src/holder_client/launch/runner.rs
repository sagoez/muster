use std::{
    collections::HashSet,
    fs, io,
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant, SystemTime},
};

use muster_domain::{
    port::{OutputSink, ProcessHandle, ProcessRunner},
    pty::{ExitOutcome, PtyError, SpawnRequest},
    value::{OutputCursor, Pid, PositiveDuration, ProcessName, ProjectRoot},
};
use muster_os::{process_identity::LocalProcessIdentity, spawn::ensure_working_dir};
use muster_protocol::{
    holder::{
        ABANDON_TIMEOUT, ClientMessage, ClientRole, HOLDER_READY_TIMEOUT, HolderConfig,
        HolderControl, HolderDirs, HolderError, HolderId, HolderLauncher, HolderMessage,
        HolderRecord, LeftExit, LegacyEntry, LineMatcher, LogLines, LogScan, LogTail, RecordScan,
        SOCKET_CHECK_INTERVAL, first_match, is_handover_file, is_socket_file, last_lines,
        read_log_tail, socket_folder_entries,
    },
    ipc,
};

use crate::holder_client::{
    connection::handle::HolderHandle,
    launch::stranded_handle::StrandedHandle,
    pool::{holder_pool::HolderPool, sweep_failure::SweepFailure, sweep_target::SweepTarget},
};

/// Multiple of the ready timeout a launched holder waits for its first
/// supervisor. Above one, and the ready timeout is positive, so a holder the
/// runner is still waiting for never gives up, and one the runner gave up on
/// always does.
const SUPERVISOR_DEADLINE_FACTOR: u32 = 2;
/// Interval between attempts to reach a launching holder's socket.
const READY_POLL: Duration = Duration::from_millis(10);
/// Most bytes of a log's end read back for a process no supervisor saw run.
const LOG_TAIL_BYTES: u64 = 64 * 1024;
/// Polls a launch may run past each of its bounds: one past the ready
/// timeout, one past the abandon timeout.
const POLLS_PAST_BOUNDS: u32 = 2;
/// Longest a launch or an adoption takes to return: the ready timeout, then
/// the abandon timeout a failed launch spends stopping its holder, or an
/// adoption waiting for a holder to bind its socket again, each overrun by
/// at most one poll.
pub(crate) const LONGEST_LAUNCH: Duration = HOLDER_READY_TIMEOUT
    .into_inner()
    .saturating_add(ABANDON_TIMEOUT)
    .saturating_add(READY_POLL.saturating_mul(POLLS_PAST_BOUNDS));

/// Runs every process under its own detached holder, so processes outlive the
/// service, and finds and adopts the holders a previous service left running.
pub struct HolderRunner {
    /// Starts each holder.
    launcher: Box<dyn HolderLauncher>,
    /// Where holders keep their sockets, records and logs.
    dirs: HolderDirs,
    /// Longest a launched holder may take to serve its supervisor.
    ready_timeout: PositiveDuration,
}

impl HolderRunner {
    /// Creates a runner launching holders through `launcher` into `dirs`. Each
    /// request carries its own launch settings: size, scrollback, log cap and
    /// colors.
    pub fn new(launcher: Box<dyn HolderLauncher>, dirs: HolderDirs) -> Self {
        Self {
            launcher,
            dirs,
            ready_timeout: HOLDER_READY_TIMEOUT,
        }
    }

    /// How long a launched holder waits for its first supervisor: past the
    /// ready timeout, so a holder the runner still waits for never gives up.
    fn supervisor_deadline(&self) -> Duration {
        self.ready_timeout
            .into_inner()
            .saturating_mul(SUPERVISOR_DEADLINE_FACTOR)
    }

    /// Shortens the ready timeout, so a test can outlast it quickly.
    #[cfg(test)]
    fn with_ready_timeout(mut self, ready_timeout: PositiveDuration) -> Self {
        self.ready_timeout = ready_timeout;
        self
    }

    /// Becomes the supervisor of the running holder `record` describes,
    /// forwarding its screen and then its output and exit to `sink`, with the
    /// output it logged after `since`, or since its run began, replayed to the
    /// sink first once the backlog is taken. A holder still running as itself
    /// by its start token whose socket is missing is waited for, up to
    /// [`ABANDON_TIMEOUT`], while it binds the socket again. A holder that is
    /// gone and left its child's exit beside its record is not reached: that
    /// exit goes to `sink` as a waiting holder would hand it over, and
    /// acknowledging it removes the record. An exit beside a holder that
    /// still runs is never taken over the holder.
    ///
    /// # Errors
    /// Returns a [`PtyError`] when the holder cannot be reached or sends no
    /// screen.
    pub fn adopt(
        &self,
        record: &HolderRecord,
        since: Option<OutputCursor>,
        sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        if let Some(left) = self.left_by_gone_holder(record) {
            return Ok(self.stranded(record, &left, sink.as_ref()));
        }
        let stream = match connect_outwaiting(record) {
            Ok(stream) => stream,
            Err(error) => {
                return match self.left_by_gone_holder(record) {
                    Some(left) => Ok(self.stranded(record, &left, sink.as_ref())),
                    None => Err(error.into()),
                };
            },
        };
        let handle = HolderHandle::supervise_replaying(
            stream,
            record.socket(),
            Some(record.log()),
            sink,
            HOLDER_READY_TIMEOUT.into_inner(),
            since,
        )?
        .with_started_at(record.child_started_at());
        Ok(Box::new(handle))
    }

    /// The handle of the process of the holder `record` names, which left
    /// the exit `left`, handed to `sink`.
    fn stranded(
        &self,
        record: &HolderRecord,
        left: &LeftExit,
        sink: &dyn OutputSink,
    ) -> Box<dyn ProcessHandle> {
        Box::new(StrandedHandle::deliver(
            record.clone(),
            self.dirs.records().clone(),
            left,
            sink,
        ))
    }

    /// The exit the holder `record` names left beside its record, unless
    /// the holder still runs: by its start token, or by its socket answering
    /// where no token tells. An exit beside a running holder, stale or put
    /// there by something else, never overrides the holder.
    fn left_by_gone_holder(&self, record: &HolderRecord) -> Option<LeftExit> {
        let left = record.left_exit(self.dirs.records())?;
        (!record.is_serving()).then_some(left)
    }

    /// Takes the exit the holder `record` names left beside its record when
    /// that holder is gone, removing the record and the exit, so no other
    /// path dials a socket nothing serves for it or reports it again.
    pub(crate) fn take_stranded(&self, record: &HolderRecord) -> Option<ExitOutcome> {
        let left = self.left_by_gone_holder(record)?;
        let _ = record.remove(self.dirs.records());
        Some(left.outcome().clone())
    }

    /// Removes the record of the holder `record` names once its exit was
    /// collected from it, so nothing of its run, an exit left beside it
    /// included, outlives the collect.
    pub(crate) fn forget_collected(&self, record: &HolderRecord) {
        let _ = record.remove(self.dirs.records());
    }

    /// The record of the holder supervising the child with `child_pid`, naming
    /// its socket and log. The record of a holder still running as itself
    /// wins over a stale one left for the same pid, even while its socket is
    /// missing as it binds it again, then one whose child outlived its
    /// holder.
    pub fn locate(&self, child_pid: Pid) -> Option<HolderRecord> {
        let candidates: Vec<HolderRecord> = HolderRecord::load_all(self.dirs.records())
            .into_iter()
            .filter(|record| record.child_pid() == child_pid)
            .collect();
        let live = candidates.iter().position(HolderRecord::is_serving);
        let surviving = || candidates.iter().position(child_runs);
        live.or_else(surviving)
            .and_then(|index| candidates.into_iter().nth(index))
    }

    /// The last bytes of the log of the process `name` of `project`, at most
    /// `LOG_TAIL_BYTES` from a line boundary, with where they start: what a
    /// supervisor that never saw the process run reports of it. None when
    /// the log cannot be read, as before any run wrote one.
    pub fn log_tail(&self, project: &ProjectRoot, name: &ProcessName) -> Option<LogTail> {
        let (at, bytes) =
            read_log_tail(&self.dirs.process_log(project, name), LOG_TAIL_BYTES).ok()?;
        Some(LogTail::builder().at(at).bytes(bytes).build())
    }

    /// The first line of the log of the process `name` of `project` that
    /// begins from `from` up to `until` and `matcher` matches, with where the
    /// scan began. None when the log cannot be read.
    pub fn log_scan(
        &self,
        project: &ProjectRoot,
        name: &ProcessName,
        from: OutputCursor,
        until: OutputCursor,
        matcher: &LineMatcher,
    ) -> Option<LogScan> {
        first_match(&self.dirs.process_log(project, name), from, until, matcher).ok()
    }

    /// The last `count` lines of the log of the process `name` of `project`
    /// that begin from `from` up to `until`, read from at most
    /// `LOG_TAIL_BYTES`. None when the log cannot be read.
    pub fn log_lines(
        &self,
        project: &ProjectRoot,
        name: &ProcessName,
        from: OutputCursor,
        until: OutputCursor,
        count: usize,
    ) -> Option<LogLines> {
        let log = self.dirs.process_log(project, name);
        last_lines(&log, from, until, count, LOG_TAIL_BYTES).ok()
    }

    /// The record of a live holder supervising the child with `child_pid`,
    /// one a supervisor can adopt now.
    pub fn serving(&self, child_pid: Pid) -> Option<HolderRecord> {
        HolderRecord::load_all(self.dirs.records())
            .into_iter()
            .find(|record| record.child_pid() == child_pid && record.accepts_now())
    }

    /// Whether `path`, in the sockets' folder, is an orphan socket: what a
    /// holder that died without unlinking it left behind. A socket is kept
    /// while `served` names its holder id, however its record spells the
    /// path, and while it was modified after `settled`, since a holder binds
    /// its socket just before it writes its record and one never supervised
    /// is gone by its first-supervisor deadline. Only sockets are orphans; a
    /// file of another kind is left alone. How a connection to a dead socket
    /// fails differs by platform, so it is never the test.
    fn is_orphan_socket(
        &self,
        path: &Path,
        served: &HashSet<HolderId>,
        settled: SystemTime,
    ) -> bool {
        self.dirs.is_socket(path)
            && is_socket_file(path)
            && !HolderId::of_file(path).is_some_and(|id| served.contains(&id))
            && modified_before(path, settled)
    }

    /// Lists the sockets' folder once, checked private first with the
    /// folder holding it, and deletes the spec and error files of launches
    /// no runner waits on, since a spec carries the child's environment and
    /// none is waiting at service start, and every orphan socket, keeping
    /// those `scan` names as serving or unreadable. A file that cannot be
    /// removed, or an entry that cannot be read, does not stop the sweep; a
    /// folder that is refused or cannot be listed is never entered and is
    /// the one failure.
    fn sweep_socket_folder(&self, scan: &RecordScan) -> Vec<SweepFailure> {
        let folder = self.dirs.sockets();
        let entries = match socket_folder_entries(folder) {
            Ok(entries) => entries,
            Err(error) => return vec![folder_failure(folder, error)],
        };
        let served = scan.kept_ids();
        let settled = SystemTime::now()
            .checked_sub(self.supervisor_deadline())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        entries
            .into_iter()
            .filter_map(|entry| match entry {
                Err(error) => Some(folder_failure(folder, error.into())),
                Ok(path) if is_handover_file(&path) => {
                    removal_failure(path, SweepTarget::HandoverFile)
                },
                Ok(path) if self.is_orphan_socket(&path, &served, settled) => {
                    removal_failure(path, SweepTarget::Socket)
                },
                Ok(_) => None,
            })
            .collect()
    }
}

impl HolderPool for HolderRunner {
    /// Reads the records of `project`, or of every project, once and probes
    /// each holder as [`HolderRecord::is_serving`] does: a reused pid fails,
    /// and a holder binding its socket again or holding an uncollected exit
    /// passes. A record whose holder no longer serves is kept while it left
    /// its child's exit, which then lands like any collected one, or while
    /// its child, the same process still, runs on, so it is adopted as
    /// unsupervised rather than started a second time; every other such
    /// record is deleted. Records of other projects are left as they are.
    fn scan_records(&self, project: Option<&ProjectRoot>) -> RecordScan {
        RecordScan::read(self.dirs.records(), project).retaining_unserved(|record| {
            let kept = child_runs(record);
            if !kept {
                let _ = record.remove(self.dirs.records());
            }
            kept
        })
    }

    /// Sweeps the sockets' folder as [`Self::sweep_socket_folder`] does,
    /// then removes every exit left beside no record, as one a holder left
    /// after its record was deleted, which nothing would report.
    fn sweep_sockets(&self, scan: &RecordScan) -> Vec<SweepFailure> {
        let mut failures = self.sweep_socket_folder(scan);
        failures.extend(
            HolderRecord::sweep_orphan_exits(self.dirs.records())
                .into_iter()
                .map(|(path, error)| {
                    SweepFailure::builder()
                        .target(SweepTarget::LeftExit)
                        .path(path)
                        .error(error.into())
                        .build()
                }),
        );
        failures
    }

    /// Sweeps the folder in the state directory where an earlier muster
    /// bound holder sockets, run at service start once the holders still
    /// serving there are adopted: removes every handover file, as
    /// [`LegacyEntry::Handover`] says why, and the socket of every holder
    /// `scan` finds no longer serving, by the rule adoption goes by, then
    /// the folder once it is empty. A folder still holding a serving
    /// holder's socket, or an unreadable record's, is left for a later
    /// start; a folder never made is nothing to do.
    fn sweep_legacy_sockets(&self, scan: &RecordScan) -> Vec<SweepFailure> {
        let folder = self.dirs.legacy_sockets();
        let entries = match self.dirs.legacy_entries() {
            Ok(entries) => entries,
            Err(error) => return vec![folder_failure(folder, error)],
        };
        let (serving, unreadable) = (scan.serving_ids(), scan.unreadable_ids());
        let mut failures: Vec<SweepFailure> = entries
            .into_iter()
            .filter_map(|entry| match entry {
                Err(error) => Some(folder_failure(folder, error.into())),
                Ok(path) => match self.dirs.legacy_entry(&path, &serving, &unreadable) {
                    LegacyEntry::Handover => removal_failure(path, SweepTarget::HandoverFile),
                    LegacyEntry::Ended => removal_failure(path, SweepTarget::Socket),
                    LegacyEntry::Running | LegacyEntry::UnreadableRecord | LegacyEntry::Stray => {
                        None
                    },
                },
            })
            .collect();
        match self.dirs.remove_legacy_sockets() {
            Err(error) if error.kind() != io::ErrorKind::DirectoryNotEmpty => {
                failures.push(folder_failure(folder, error.into()));
            },
            _ => {},
        }
        failures
    }

    /// Removes the record of the holder `record` names, with the exit it
    /// left beside it.
    fn forget_stranded(&self, record: &HolderRecord) -> io::Result<()> {
        record.remove(self.dirs.records())
    }

    /// The log every run of the process `name` of `project` appends to.
    fn process_log(&self, project: &ProjectRoot, name: &ProcessName) -> PathBuf {
        self.dirs.process_log(project, name)
    }

    /// Removes the per-holder logs an earlier muster kept.
    fn remove_legacy_logs(&self) -> io::Result<()> {
        self.dirs.remove_legacy_logs()
    }
}

/// The failure of a sweep to enter, list or remove the folder `folder`.
fn folder_failure(folder: &Path, error: HolderError) -> SweepFailure {
    SweepFailure::builder()
        .target(SweepTarget::Folder)
        .path(folder.to_path_buf())
        .error(error)
        .build()
}

/// Removes the `target` file at `path`, giving it back with why when it
/// cannot be.
fn removal_failure(path: PathBuf, target: SweepTarget) -> Option<SweepFailure> {
    fs::remove_file(&path).err().map(|error| {
        SweepFailure::builder()
            .target(target)
            .path(path)
            .error(error.into())
            .build()
    })
}

impl ProcessRunner for HolderRunner {
    /// Launches a holder for `request`, waits up to [`HOLDER_READY_TIMEOUT`]
    /// for it to serve, and supervises it. A request the holder would refuse
    /// fails here, before anything is launched. A holder that reports its own
    /// failure is left to finish; any other failure abandons it, and a reason
    /// it reports meanwhile replaces the runner's own error. A launch returns
    /// within [`LONGEST_LAUNCH`]: [`HOLDER_READY_TIMEOUT`] to serve, then, on
    /// a failure, [`ABANDON_TIMEOUT`] to stop the holder, each overrun by at
    /// most one poll.
    ///
    /// # Errors
    /// Returns a [`PtyError`] when the working directory is invalid, the
    /// request names no project, the socket directory is not private, or the
    /// holder cannot be launched or does not become ready.
    fn spawn(
        &self,
        request: SpawnRequest,
        sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        ensure_working_dir(&request)?;
        let Some(project) = request
            .project()
            .clone()
            .and_then(|root| ProjectRoot::try_new(root).ok())
        else {
            return Err(HolderError::MissingProject.into());
        };
        let log = self.dirs.process_log(&project, request.process());
        self.dirs.prepare_sockets()?;
        let id = HolderId::generate().map_err(|error| PtyError::System(error.to_string()))?;
        let socket = self.dirs.socket_path(&id);
        let config = HolderConfig::builder()
            .log(log.clone())
            .id(id)
            .request(request)
            .socket(socket.clone())
            .record_dir(self.dirs.records().clone())
            .supervisor_deadline(self.supervisor_deadline())
            .socket_check_interval(SOCKET_CHECK_INTERVAL)
            .build();
        let error_file = config.error_path();
        self.launcher.launch(config)?;
        let supervised = match self.supervise_launched(&socket, &log, &error_file, sink) {
            Err(error @ HolderError::FailedToStart { .. }) => Err(error),
            Err(error) => {
                abandon(&socket);
                Err(reported_failure(&socket, &error_file).unwrap_or(error))
            },
            Ok(handle) => Ok(handle),
        };
        let _ = fs::remove_file(&error_file);
        Ok(Box::new(supervised?.with_backlog()))
    }
}

impl HolderRunner {
    /// Becomes the supervisor of a holder just launched at `socket`, logging
    /// to `log`, within the ready timeout.
    ///
    /// # Errors
    /// Returns [`HolderError::FailedToStart`] with the holder's own error when
    /// it reports one, [`HolderError::NotReady`] when it does not serve in
    /// time, or the error from greeting it.
    fn supervise_launched(
        &self,
        socket: &Path,
        log: &Path,
        error_file: &Path,
        sink: Box<dyn OutputSink>,
    ) -> Result<HolderHandle, HolderError> {
        let deadline = Instant::now() + self.ready_timeout.into_inner();
        let stream = connect_by(socket, error_file, deadline)?;
        let remaining = deadline.saturating_duration_since(Instant::now());
        HolderHandle::supervise(stream, socket, Some(log), sink, remaining.max(READY_POLL))
    }
}

/// Stops a holder the runner gave up on: retries reaching it, then supervises
/// it, kills its child and collects the exit, so it finishes, all within
/// [`ABANDON_TIMEOUT`]. A holder still unreachable then stops itself at its
/// supervisor deadline.
pub(crate) fn abandon(socket: &Path) {
    finish(socket, true);
}

/// Reaches the holder at `socket` within [`ABANDON_TIMEOUT`], so a holder
/// binding its socket again is still reached, killing its child first when
/// `kill` is set, and collects the exit. Returns the exit collected, or none
/// when the holder could not be reached or did not hand it over in time.
pub(crate) fn finish(socket: &Path, kill: bool) -> Option<ExitOutcome> {
    let deadline = Instant::now() + ABANDON_TIMEOUT;
    loop {
        match ipc::connect(socket) {
            Ok(mut stream) => return collect_exit(&mut stream, socket, deadline, kill).ok(),
            Err(_) if Instant::now() >= deadline => return None,
            Err(_) => thread::sleep(READY_POLL),
        }
    }
}

/// Supervises the holder on `stream`, kills its child when `kill` is set,
/// and collects the exit, all before `deadline`. Returns the exit.
///
/// # Errors
/// Returns a [`HolderError`] when the exchange fails or the exit does not
/// arrive before the deadline.
fn collect_exit(
    stream: &mut UnixStream,
    socket: &Path,
    deadline: Instant,
    kill: bool,
) -> Result<ExitOutcome, HolderError> {
    let remaining = || {
        Some(deadline.saturating_duration_since(Instant::now()))
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| HolderError::NotReady(socket.to_path_buf()))
    };
    stream.set_write_timeout(Some(remaining()?))?;
    ClientMessage::Hello {
        role: ClientRole::Supervisor,
    }
    .write_to(stream)?;
    if kill {
        ClientMessage::Control(HolderControl::Kill).write_to(stream)?;
    }
    let outcome = loop {
        stream.set_read_timeout(Some(remaining()?))?;
        if let HolderMessage::Exited(outcome) = HolderMessage::read_from(stream)? {
            break outcome;
        }
    };
    ClientMessage::CollectExit.write_to(stream)?;
    Ok(outcome)
}

/// Connects to the holder `record` names. While the holder still runs as
/// itself by its start token and its socket is missing or refuses, as while
/// it binds the socket again after something removed it, the connect is
/// tried again for up to [`ABANDON_TIMEOUT`], several of its socket checks.
///
/// # Errors
/// Returns the I/O error of the last try.
fn connect_outwaiting(record: &HolderRecord) -> io::Result<UnixStream> {
    let deadline = Instant::now() + ABANDON_TIMEOUT;
    loop {
        match ipc::connect(record.socket()) {
            Ok(stream) => return Ok(stream),
            Err(error)
                if !matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                ) || Instant::now() >= deadline
                    || record.alive_by_token() != Some(true) =>
            {
                return Err(error);
            },
            Err(_) => thread::sleep(READY_POLL),
        }
    }
}

/// Whether the child `record` names still runs as the same process, as one
/// that ignores the hangup does after its holder died, and in the boot it
/// was recorded in.
fn child_runs(record: &HolderRecord) -> bool {
    LocalProcessIdentity::may_be_this_boot(record.boot_id().as_ref())
        && LocalProcessIdentity::is_same_process(record.child_pid(), record.child_start_token())
}

/// Whether the file at `path` was last modified before `instant`. One whose
/// time cannot be read counts as recent, so it is kept.
fn modified_before(path: &Path, instant: SystemTime) -> bool {
    fs::symlink_metadata(path)
        .and_then(|metadata| metadata.modified())
        .is_ok_and(|modified| modified < instant)
}

/// Connects to a launching holder's socket, retrying while it is not bound
/// yet, until `deadline`. Stops early when the holder reports in `error_file`
/// that it failed, and reads that file once more when the deadline passes.
///
/// # Errors
/// Returns [`HolderError::FailedToStart`] with the holder's error,
/// [`HolderError::NotReady`] when the deadline passes first, or an I/O error
/// other than the socket being absent or not yet accepting.
fn connect_by(
    socket: &Path,
    error_file: &Path,
    deadline: Instant,
) -> Result<UnixStream, HolderError> {
    loop {
        if let Some(failure) = reported_failure(socket, error_file) {
            return Err(failure);
        }
        match ipc::connect(socket) {
            Ok(stream) => return Ok(stream),
            Err(error)
                if !matches!(
                    error.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                ) =>
            {
                return Err(error.into());
            },
            Err(_) if Instant::now() >= deadline => {
                return Err(reported_failure(socket, error_file)
                    .unwrap_or_else(|| HolderError::NotReady(socket.to_path_buf())));
            },
            Err(_) => thread::sleep(READY_POLL),
        }
    }
}

/// The failure the holder for `socket` reported in `error_file`, if it has
/// written one.
fn reported_failure(socket: &Path, error_file: &Path) -> Option<HolderError> {
    fs::read_to_string(error_file)
        .ok()
        .filter(|reason| !reason.is_empty())
        .map(|reason| HolderError::FailedToStart {
            socket: socket.to_path_buf(),
            reason,
        })
}

#[cfg(test)]
mod tests {
    use std::{
        fs::{self, Permissions},
        os::unix::{
            fs::{PermissionsExt, symlink},
            net::{UnixDatagram, UnixListener},
        },
        process,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        thread::JoinHandle,
    };

    use crossbeam_channel::{Receiver, bounded, unbounded};
    use muster_domain::{
        color_scheme::ColorScheme,
        process::{LogPattern, PatternKind, StopSignal},
        pty::{ExitCause, ProcessOutput, PtySize, ScreenSource},
        value::{BootId, Cols, CommandLine, ProcessName, Rows, StartToken},
    };
    use muster_holder::{
        channel_sink::ChannelSink,
        testing::{TEST_COLS, TEST_ROWS, TEST_SCROLLBACK, test_request, test_stop},
        thread_launcher::ThreadLauncher,
    };
    use muster_os::{
        private_dir::PRIVATE_DIR_MODE,
        signal_target::SignalTarget,
        test_scratch::SHARED_DIR_MODE,
        test_wait::{POLL, WAIT, wait_until},
    };
    use muster_protocol::holder::{
        LineMatcher, RECORD_EXTENSION, SOCKET_EXTENSION, SPEC_EXTENSION, ServedScreen,
        client::{greet, screen_text},
        ensure_socket_folder, error_path_beside,
    };
    use muster_terminal::{
        TerminalEmulator,
        escape::{RUN_RESET_AT, RUN_START},
        log_renderer::LogRenderer,
    };
    use rustix::{net::listen, process::Signal};

    use super::*;
    use crate::{
        holder_client::{
            connection::{losing_sink::LosingSink, watch::Watch},
            launch::{runner_fixture::Fixture, sabotage::Sabotage, spec_launcher::CORRUPT_SPEC},
        },
        supervisor::HolderAdopter,
    };

    /// Where the run whose exit a test leaves began in its log.
    const LEFT_RUN_START: u64 = 7;
    /// What makes a boot id another boot's.
    const EARLIER_BOOT_SUFFIX: &str = "-earlier";
    /// A project folder other than the fixture's.
    const OTHER_PROJECT: &str = "other";

    /// Longest one screen probe waits for the holder, far below [`WAIT`] so a
    /// polling loop stays within its bound.
    const PROBE_TIMEOUT: Duration = Duration::from_millis(200);
    /// The line a replayed child prints first.
    const REPLAY_MARKER: &str = "REPLAY-MARKER";
    /// The line a replayed child prints last.
    const REPLAY_DONE: &str = "REPLAY-DONE";
    /// Bytes the replayed child prints between them: past what one backlog
    /// read used to hold.
    const REPLAY_FLOOD_BYTES: usize = 1024 * 1024;
    /// Bytes of one line the replayed child prints, past one piece of a log
    /// scan.
    const LONG_LINE_BYTES: usize = 100 * 1024;
    /// How long a test watches for a replay held back until the backlog is
    /// taken.
    const HELD_REPLAY: Duration = Duration::from_millis(300);
    /// PTY rows of every test child.
    const ROWS: u16 = TEST_ROWS;
    /// PTY cols of every test child.
    const COLS: u16 = TEST_COLS;
    /// Numbered lines the heavy-output child prints.
    const HEAVY_LINES: usize = 20_000;
    /// Line the heavy-output child must have reached before the runner drops.
    const HEAVY_DROP_AT: &str = "line-2000";
    /// The heavy-output child's last line, whole, so its scroll has happened:
    /// waiting for `DONE` alone can see the text before the newline that
    /// scrolls the screen.
    const HEAVY_DONE_LINE: &str = "DONE\r\n";
    /// Lines between the heavy-output child's short pauses, which stretch its
    /// output across the supervisor's departure and the adoption.
    const HEAVY_PAUSE_EVERY: usize = 1000;
    /// Length of each pause, in seconds as `sleep` reads it.
    const HEAVY_PAUSE: &str = "0.05";
    /// Stale records saved beside a live one for the same child, after it,
    /// so a directory listing is unlikely to name the live one first.
    const STALE_RECORDS: usize = 4;
    /// Grace given to a child that ignores the stop signal.
    const STOP_GRACE: Duration = Duration::from_millis(500);
    /// A link beside the sockets' folder, leading to it.
    const LINKED_SOCKETS: &str = "linked-sock";
    /// A folder a link planted in place of the sockets folder leads to.
    const PLANTED_TARGET: &str = "planted";
    /// Ready timeout of a runner a test makes give up quickly.
    const SHORT_READY: PositiveDuration =
        match PositiveDuration::try_new(Duration::from_millis(300)) {
            Ok(timeout) => timeout,
            Err(_) => panic!("the short ready timeout must be positive"),
        };
    /// How long a slow holder takes to start: past [`SHORT_READY`].
    const SLOW_START: Duration = Duration::from_millis(600);
    /// How long a holder an abandon races takes to start: well inside
    /// [`ABANDON_TIMEOUT`].
    const STARTING_DELAY: Duration = Duration::from_millis(300);
    /// A first-supervisor deadline past any test's length.
    const LONG_SUPERVISOR_DEADLINE: Duration = Duration::from_secs(60);
    /// A regular file the bad-log test makes the log directory's parent.
    const NOT_A_DIRECTORY: &str = "not-a-dir";
    /// A handover name the sweep cannot remove, made a directory.
    const STUCK_HANDOVER: &str = "stuck.json";
    /// File the pid-reporting child writes its pid to.
    const PID_FILE: &str = "child.pid";
    /// Time a surviving child would need to write its pid file late.
    const PID_FILE_GRACE: Duration = Duration::from_millis(500);

    /// Whether the holder at `socket` serves its screen at `size`. A holder
    /// applies one connection's frames in order, so once it serves at a size a
    /// supervisor sent, it has applied everything that supervisor sent before.
    fn serves_at(socket: &Path, size: PtySize) -> bool {
        let Ok(mut stream) = ipc::connect(socket) else {
            return false;
        };
        greet(
            &mut stream,
            socket,
            ClientRole::Attach { read_only: true },
            PROBE_TIMEOUT,
        )
        .is_ok_and(|greeting| greeting.size() == size)
    }

    #[test]
    fn spawned_process_streams_output_through_the_sink() {
        let fixture = Fixture::new();
        let runner = fixture.runner();
        let (sink, mut watch) = Watch::new();
        let mut handle = runner
            .spawn(fixture.request("printf hello; read x"), sink)
            .unwrap();
        watch.connected(&mut handle);
        watch.until("the child's output", |watch| watch.text().contains("hello"));

        let pid = handle.process_id().unwrap();
        let record = runner.locate(pid).unwrap();
        assert_eq!(
            record.socket().parent(),
            Some(fixture.dirs().sockets().as_path())
        );
        assert!(fixture.log(&record).contains("hello"));

        handle.write_input(b"\n").unwrap();
        watch.until("the child's exit", |watch| watch.exit().is_some());
        assert_eq!(watch.exit(), Some(ExitOutcome::Succeeded));
        handle.acknowledge_exit().unwrap();
        wait_until("the holder to finish once the exit is collected", || {
            fixture.record_files().is_empty()
        });
    }

    /// An exit the supervisor never received stays with the holder: its record
    /// survives and a later adoption still sees the exit, which only an
    /// acknowledgement lets go of.
    #[test]
    fn an_exit_the_supervisor_never_handled_is_held_for_a_later_adoption() {
        let fixture = Fixture::new();
        let runner = fixture.runner();
        let reported = Arc::new(AtomicBool::new(false));
        let mut handle = runner
            .spawn(
                fixture.request("read x"),
                Box::new(LosingSink::new(Arc::clone(&reported))),
            )
            .unwrap();
        let record = runner.locate(handle.process_id().unwrap()).unwrap();

        handle.write_input(b"\n").unwrap();
        wait_until("the holder to report the exit", || {
            reported.load(Ordering::SeqCst)
        });
        let probe = PtySize::try_new(ROWS + 1, COLS).unwrap();
        handle
            .resize(probe)
            .expect("the holder keeps serving its supervisor after the exit");
        wait_until("the holder to apply the resize sent after the exit", || {
            serves_at(record.socket(), probe)
        });
        drop(handle);
        assert_eq!(
            fixture.record_files(),
            vec![record.clone()],
            "the holder keeps the unacknowledged exit"
        );

        let runner = fixture.runner();
        assert_eq!(runner.scan_records(None).live(), vec![record.clone()]);
        let (sink, mut watch) = Watch::new();
        let mut adopted = runner.adopt(&record, None, sink).unwrap();
        watch.connected(&mut adopted);
        watch.until("the held exit", |watch| watch.exit().is_some());
        assert_eq!(watch.exit(), Some(ExitOutcome::Succeeded));
        adopted.acknowledge_exit().unwrap();
        wait_until("the holder to finish once the exit is acknowledged", || {
            fixture.record_files().is_empty()
        });
    }

    /// An acknowledgement reaches the holder even when the handle is dropped
    /// straight after it, so the holder finishes and removes its record.
    #[test]
    fn an_acknowledged_exit_lets_the_holder_finish_when_the_handle_drops_at_once() {
        let fixture = Fixture::new();
        let runner = fixture.runner();
        let (sink, mut watch) = Watch::new();
        let mut handle = runner.spawn(fixture.request("read x"), sink).unwrap();

        handle.write_input(b"\n").unwrap();
        watch.until("the child's exit", |watch| watch.exit().is_some());
        handle.acknowledge_exit().unwrap();
        drop(handle);

        wait_until("the holder to finish and remove its record", || {
            fixture.record_files().is_empty()
        });
    }

    /// The pattern `text` as a regex.
    fn regex(text: &str) -> LogPattern {
        LineMatcher::checked(text, PatternKind::Regex).unwrap()
    }

    /// An adopting handle reads back the lines asked for that the child
    /// logged before it connected, from the run's start or from a given
    /// cursor, so no wanted line falls between the earlier connection and
    /// the new stream.
    #[test]
    fn an_adoption_reads_back_what_was_logged_before_it_connected() {
        let fixture = Fixture::new();
        let runner = fixture.runner();
        let (sink, mut watch) = Watch::new();
        let handle = runner
            .spawn(fixture.request("echo first; echo second; sleep 600"), sink)
            .unwrap();
        let start = handle.run_start().unwrap();
        drop(handle);
        let record = runner.scan_records(None).live().remove(0);
        wait_until("both lines in the log", || {
            fixture.log(&record).contains("second")
        });
        watch.drain();

        let (sender, outputs) = unbounded();
        let mut whole = runner
            .adopt(&record, None, Box::new(ChannelSink::new(sender)))
            .unwrap();
        assert_eq!(whole.take_backlog(), None, "replayed through the sink");
        let lines = [regex("first|second")];
        assert!(whole.replay_matching(&lines).is_some());
        let reset = &RUN_START[RUN_RESET_AT..];
        assert_eq!(
            replayed(&outputs, 2),
            (start, [reset, b"first\r\nsecond\r\n"].concat()),
            "the run starts at its reset, which its first line begins with"
        );
        drop(whole);

        let later = start.advanced((reset.len() + "first\r\n".len()) as u64);
        let (sender, outputs) = unbounded();
        let mut rest = runner
            .adopt(&record, Some(later), Box::new(ChannelSink::new(sender)))
            .unwrap();
        let _ = rest.replay_matching(&lines);
        assert_eq!(replayed(&outputs, 1), (later, b"second\r\n".to_vec()));
        rest.kill().unwrap();
    }

    /// An adopting handle asked for no lines reads nothing back: no logged
    /// output is forwarded, and it says where its stream begins, the log's
    /// end as it connected.
    #[test]
    fn an_adoption_asked_for_no_lines_replays_nothing() {
        let fixture = Fixture::new();
        let runner = fixture.runner();
        let (sink, _watch) = Watch::new();
        let handle = runner
            .spawn(fixture.request("echo first; echo second; sleep 600"), sink)
            .unwrap();
        drop(handle);
        let record = runner.scan_records(None).live().remove(0);
        wait_until("both lines in the log", || {
            fixture.log(&record).contains("second")
        });

        let (sender, outputs) = unbounded();
        let mut adopted = runner
            .adopt(&record, None, Box::new(ChannelSink::new(sender)))
            .unwrap();
        let end = adopted.output_end();

        assert_eq!(adopted.replay_matching(&[]), end);
        thread::sleep(HELD_REPLAY);
        let placed = outputs
            .try_iter()
            .find(|output| matches!(output, ProcessOutput::Placed { .. }));
        assert!(placed.is_none(), "replayed {placed:?}");
        adopted.kill().unwrap();
    }

    /// An adopting handle replays nothing until it is told what to look for,
    /// then scans the whole run back to a line far past what one read used to
    /// hold and forwards only the lines asked for, never the flood between
    /// them, so readiness and waits still see a line printed long ago.
    #[test]
    fn an_adoption_forwards_only_the_lines_asked_for_from_far_back() {
        let fixture = Fixture::new();
        let runner = fixture.runner();
        let command = format!(
            "echo {REPLAY_MARKER}; head -c {REPLAY_FLOOD_BYTES} /dev/zero | tr '\\0' x; \
             echo; echo {REPLAY_DONE}; sleep 600"
        );
        let (sink, _watch) = Watch::new();
        let handle = runner.spawn(fixture.request(&command), sink).unwrap();
        drop(handle);
        let record = runner.scan_records(None).live().remove(0);
        wait_until("the flood in the log", || {
            fixture.log(&record).contains(REPLAY_DONE)
        });

        let (sender, outputs) = unbounded();
        let mut adopted = runner
            .adopt(&record, None, Box::new(ChannelSink::new(sender)))
            .unwrap();
        let _ = adopted.take_backlog();
        assert!(
            outputs.recv_timeout(HELD_REPLAY).is_err(),
            "nothing is replayed before the handle is told what to look for"
        );
        let _ = adopted.replay_matching(&[regex(&format!("{REPLAY_MARKER}|{REPLAY_DONE}"))]);

        let mut lines = Vec::new();
        while !lines.iter().any(|line: &String| line.contains(REPLAY_DONE)) {
            if let ProcessOutput::Placed { bytes, .. } = outputs.recv_timeout(WAIT).unwrap() {
                lines.push(String::from_utf8_lossy(&bytes).into_owned());
            }
        }
        assert_eq!(lines.len(), 2, "only the lines asked for: {lines:?}");
        assert!(lines[0].contains(REPLAY_MARKER), "{lines:?}");
        adopted.kill().unwrap();
    }

    /// A matched line longer than one piece of a log scan, whose match lies
    /// in its first piece, is replayed whole and contiguous from where it
    /// began, so the tap renders it as the log does and still sees the
    /// match.
    #[test]
    fn a_long_matched_line_is_replayed_whole() {
        let fixture = Fixture::new();
        let runner = fixture.runner();
        let command = format!(
            "printf '\\033[32m{REPLAY_MARKER} '; head -c {LONG_LINE_BYTES} /dev/zero | tr '\\0' x; \
             printf '\\033[0m\\n'; echo {REPLAY_DONE}; sleep 600"
        );
        let (sink, _watch) = Watch::new();
        let handle = runner.spawn(fixture.request(&command), sink).unwrap();
        let start = handle.run_start().unwrap();
        drop(handle);
        let record = runner.scan_records(None).live().remove(0);
        wait_until("the long line in the log", || {
            fixture.log(&record).contains(REPLAY_DONE)
        });

        let (sender, outputs) = unbounded();
        let mut adopted = runner
            .adopt(&record, None, Box::new(ChannelSink::new(sender)))
            .unwrap();
        let _ = adopted.take_backlog();
        let _ = adopted.replay_matching(&[regex(REPLAY_MARKER)]);
        let (at, bytes) = replayed(&outputs, 1);

        assert_eq!(at, start, "replayed from where the line began");
        let mut renderer = LogRenderer::new();
        renderer.advance(&bytes);
        let lines = renderer.take_lines();
        assert!(
            lines[0].plain().starts_with(REPLAY_MARKER),
            "{} bytes replayed",
            bytes.len()
        );
        adopted.kill().unwrap();
    }

    /// The first `count` pieces of output an adopting handle replayed to
    /// `outputs`, joined, with where the first began.
    fn replayed(outputs: &Receiver<ProcessOutput>, count: usize) -> (OutputCursor, Vec<u8>) {
        let mut start = None;
        let mut bytes = Vec::new();
        let mut lines = 0;
        while lines < count {
            if let ProcessOutput::Placed { at, bytes: piece } = outputs.recv_timeout(WAIT).unwrap()
            {
                start.get_or_insert(at);
                lines += piece.iter().filter(|byte| **byte == b'\n').count();
                bytes.extend_from_slice(&piece);
            }
        }
        (start.unwrap(), bytes)
    }

    #[test]
    fn dropping_the_runner_leaves_the_process_running_and_adopt_resumes_output() {
        let fixture = Fixture::new();
        let runner = fixture.runner();
        let (sink, mut watch) = Watch::new();
        let mut handle = runner
            .spawn(
                fixture.request("while :; do echo tick; sleep 0.05; done"),
                sink,
            )
            .unwrap();
        watch.connected(&mut handle);
        watch.until("a tick", |watch| watch.text().contains("tick"));
        let pid = handle.process_id().unwrap();
        drop(handle);
        drop(runner);

        let runner = fixture.runner();
        let records = runner.scan_records(None).live();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].child_pid(), pid);

        let (sink, mut watch) = Watch::new();
        let mut adopted = runner.adopt(&records[0], None, sink).unwrap();
        watch.connected(&mut adopted);
        assert_eq!(adopted.process_id(), Some(pid));
        watch.until("a tick after the adoption snapshot", |watch| {
            watch
                .chunks()
                .iter()
                .skip(1)
                .any(|chunk| String::from_utf8_lossy(chunk).contains("tick"))
        });
        assert!(watch.exit().is_none(), "the adopted child is still running");

        adopted.kill().unwrap();
        watch.until("the child's exit", |watch| watch.exit().is_some());
    }

    #[test]
    fn adoption_after_heavy_output_rebuilds_the_exact_screen_and_log_has_no_gap() {
        let fixture = Fixture::new();
        let runner = fixture.runner();
        let (sink, _watch) = Watch::new();
        let command = format!(
            "i=0; while [ $i -lt {HEAVY_LINES} ]; do echo line-$i; i=$((i+1)); \
             [ $((i % {HEAVY_PAUSE_EVERY})) -eq 0 ] && sleep {HEAVY_PAUSE}; done; \
             read go; echo DONE; read x"
        );
        let handle = runner.spawn(fixture.request(&command), sink).unwrap();
        let record = runner.locate(handle.process_id().unwrap()).unwrap();
        wait_until("the output to be under way", || {
            fixture.log(&record).contains(HEAVY_DROP_AT)
        });
        drop(handle);
        drop(runner);
        assert!(
            !fixture.log(&record).contains("DONE"),
            "the supervisor leaves mid-stream"
        );

        let runner = fixture.runner();
        let records = runner.scan_records(None).live();
        assert_eq!(records, vec![record.clone()]);
        let (sink, mut watch) = Watch::new();
        let mut adopted = runner.adopt(&record, None, sink).unwrap();
        watch.connected(&mut adopted);
        adopted.write_input(b"\n").unwrap();
        wait_until("the child to finish its output", || {
            fixture.log(&record).contains(HEAVY_DONE_LINE)
        });

        let log = fs::read(record.log()).unwrap();
        let output = log.strip_prefix(RUN_START).expect("the run starts the log");
        let lines = String::from_utf8_lossy(output)
            .lines()
            .map(|line| line.trim_end_matches('\r').to_string())
            .filter(|line| line.starts_with("line-"))
            .collect::<Vec<_>>();
        let expected = (0..HEAVY_LINES)
            .map(|index| format!("line-{index}"))
            .collect::<Vec<_>>();
        assert!(lines == expected, "the log has a gap or a duplicate");

        let mut replayed = TerminalEmulator::new(
            Rows::try_new(ROWS).unwrap(),
            Cols::try_new(COLS).unwrap(),
            TEST_SCROLLBACK,
            ColorScheme::Dark,
        );
        replayed.process(&log);
        let holder_screen = replayed.screen_text();
        assert_eq!(screen_text(record.socket()).unwrap(), holder_screen);
        watch.until("the adopted screen to match the holder's", |watch| {
            watch.screen().screen_text() == holder_screen
        });

        adopted.kill().unwrap();
        watch.until("the child's exit", |watch| watch.exit().is_some());
    }

    #[test]
    fn a_request_the_holder_would_refuse_fails_before_anything_launches() {
        let fixture = Fixture::new();
        let runner = fixture.runner();
        let missing_dir = test_request(
            "dev server",
            "true",
            Some(fixture.scratch().path().to_path_buf()),
            Some(fixture.scratch().path().join("absent")),
        );
        let no_project = test_request("dev server", "true", None, None);

        let (sink, _watch) = Watch::new();
        assert!(matches!(
            runner.spawn(missing_dir, sink),
            Err(PtyError::InvalidWorkingDir(_))
        ));
        let (sink, _watch) = Watch::new();
        assert!(runner.spawn(no_project, sink).is_err());
        assert!(
            fixture.holders().lock().unwrap().is_empty(),
            "nothing launched"
        );
    }

    /// Saves a record whose holder pid is the test process's own, alive but
    /// not a holder, as a reused pid would be, for a child running as
    /// `child_pid` that started with `child_start_token`.
    fn save_reused_pid_record(
        fixture: &Fixture,
        child_pid: Pid,
        child_start_token: Option<StartToken>,
    ) -> HolderRecord {
        let id = HolderId::generate().unwrap();
        let socket = fixture.dirs().socket_path(&id);
        save_record(fixture, id, socket, child_pid, child_start_token)
    }

    /// Saves a record of holder `id` at `socket` whose holder and child are
    /// the test process itself, so both count as alive.
    fn save_own_record(fixture: &Fixture, id: HolderId, socket: PathBuf) -> HolderRecord {
        save_record(fixture, id, socket, own_pid(), own_start_token())
    }

    /// Saves a record of holder `id` at `socket`, held by the test process,
    /// of the child `child_pid` with `child_start_token`.
    fn save_record(
        fixture: &Fixture,
        id: HolderId,
        socket: PathBuf,
        child_pid: Pid,
        child_start_token: Option<StartToken>,
    ) -> HolderRecord {
        let record = HolderRecord::builder()
            .socket(socket)
            .log(fixture.dirs().process_log(
                &fixture.root(),
                &ProcessName::try_new("dev server").unwrap(),
            ))
            .id(id)
            .project(fixture.root())
            .process(ProcessName::try_new("dev server").unwrap())
            .command(CommandLine::try_new("true").unwrap())
            .stop(test_stop())
            .holder_pid(Pid::try_new(process::id()).unwrap())
            .child_pid(child_pid)
            .child_start_token(child_start_token)
            .child_started_at(SystemTime::now())
            .build();
        record.save(fixture.dirs().records()).unwrap();
        record
    }

    /// The test process's own pid.
    fn own_pid() -> Pid {
        Pid::try_new(process::id()).unwrap()
    }

    /// The test process's own start token.
    fn own_start_token() -> Option<StartToken> {
        LocalProcessIdentity::start_token(own_pid())
    }

    /// A start token the test process does not have.
    fn foreign_start_token() -> Option<StartToken> {
        own_start_token().and_then(|token| StartToken::try_new(token.into_inner() + 1).ok())
    }

    #[test]
    fn stale_record_with_reused_pid_is_removed_not_adopted() {
        let fixture = Fixture::new();
        let record = save_reused_pid_record(&fixture, own_pid(), foreign_start_token());
        fixture.dirs().prepare_sockets().unwrap();
        // A datagram socket refuses every stream connect even when a child
        // another test forks inherits it; a dropped stream listener would
        // keep accepting in such a child until it execs.
        let _refusing = UnixDatagram::bind(record.socket()).unwrap();

        assert!(fixture.runner().scan_records(None).live().is_empty());
        assert!(
            fixture.record_files().is_empty(),
            "a record whose socket refuses is deleted"
        );
    }

    #[test]
    fn a_record_whose_child_is_gone_and_socket_absent_is_removed() {
        let fixture = Fixture::new();
        let mut exited = process::Command::new("true").spawn().unwrap();
        let dead_pid = Pid::try_new(exited.id()).unwrap();
        exited.wait().unwrap();
        save_reused_pid_record(&fixture, dead_pid, None);

        assert!(fixture.runner().scan_records(None).live().is_empty());
        assert!(
            fixture.record_files().is_empty(),
            "a record with no socket is deleted"
        );
    }

    /// A dead holder's record whose child, the same process still, runs on
    /// is kept and located, so the child is adopted as unsupervised rather
    /// than started twice; a live holder's record for the same pid wins.
    #[test]
    fn a_dead_holders_record_is_kept_while_its_child_runs_and_a_live_one_wins() {
        let fixture = Fixture::new();
        let orphaned = save_reused_pid_record(&fixture, own_pid(), own_start_token());

        assert_eq!(fixture.runner().scan_records(None).live(), vec![
            orphaned.clone()
        ]);
        assert_eq!(fixture.runner().locate(own_pid()), Some(orphaned));
        forget_records(&fixture);

        let fixture = Fixture::new();
        fixture.dirs().prepare_sockets().unwrap();
        let live = save_reused_pid_record(&fixture, own_pid(), own_start_token());
        let _serving = UnixListener::bind(live.socket()).unwrap();
        for _ in 0..STALE_RECORDS {
            save_reused_pid_record(&fixture, own_pid(), own_start_token());
        }
        assert_eq!(fixture.runner().locate(own_pid()), Some(live));
        forget_records(&fixture);
    }

    /// A holder too busy to accept, its backlog full, is probed in bounded
    /// time and counted as running, so a probe never stalls the service and
    /// a busy holder's record is never deleted.
    #[test]
    fn a_holder_too_busy_to_accept_is_probed_in_bounded_time_and_kept_live() {
        let fixture = Fixture::new();
        fixture.dirs().prepare_sockets().unwrap();
        let record = save_reused_pid_record(&fixture, own_pid(), foreign_start_token());
        let listener = UnixListener::bind(record.socket()).unwrap();
        listen(&listener, 0).unwrap();
        let _queued = UnixStream::connect(record.socket()).unwrap();

        let (sender, probed) = bounded(1);
        let probe = record.clone();
        thread::spawn(move || {
            let _ = sender.send(probe.is_serving());
        });

        assert_eq!(probed.recv_timeout(WAIT), Ok(true));
        assert_eq!(fixture.runner().scan_records(None).live(), vec![record]);
        forget_records(&fixture);
    }

    /// Deletes the fixture's stand-in records, which name no holder, so its
    /// drop does not wait out an abandon of each.
    fn forget_records(fixture: &Fixture) {
        fs::remove_dir_all(fixture.dirs().records()).unwrap();
    }

    /// A settled socket no live holder's record names is swept, even one
    /// something still accepts on, and whatever a connection to it would
    /// say; one a live holder's record names is kept, and so is one too young
    /// to have its record yet.
    #[test]
    fn the_sweep_removes_orphan_sockets_by_record_and_age() {
        let fixture = Fixture::new();
        fixture.dirs().prepare_sockets().unwrap();
        let runner = fixture.runner().with_ready_timeout(SHORT_READY);
        let orphan = fixture.dirs().socket_path(&HolderId::generate().unwrap());
        drop(UnixListener::bind(&orphan).unwrap());
        let accepting = fixture.dirs().socket_path(&HolderId::generate().unwrap());
        let _accepting = UnixListener::bind(&accepting).unwrap();
        let served = save_reused_pid_record(&fixture, own_pid(), own_start_token());
        let _serving = UnixListener::bind(served.socket()).unwrap();
        let dead_holder = save_dead_holder_record(&fixture);
        drop(UnixListener::bind(dead_holder.socket()).unwrap());
        thread::sleep(SHORT_READY.into_inner() * SUPERVISOR_DEADLINE_FACTOR + POLL);
        let young = fixture.dirs().socket_path(&HolderId::generate().unwrap());
        drop(UnixListener::bind(&young).unwrap());

        assert!(swept(&runner).is_empty());

        assert!(!orphan.exists(), "an unnamed socket is removed");
        assert!(!accepting.exists(), "accepting is no proof of a holder");
        assert!(
            !dead_holder.socket().exists(),
            "a dead holder's socket goes"
        );
        assert!(served.socket().exists(), "a live holder's socket is kept");
        assert!(
            young.exists(),
            "a socket younger than a holder's start stays"
        );
        forget_records(&fixture);
    }

    /// A live holder whose record spells its socket through a link to the
    /// sockets' folder keeps its socket: the sweep goes by holder id, not
    /// by how the record spells the path.
    #[test]
    fn the_sweep_keeps_a_live_socket_its_record_spells_through_a_link() {
        let fixture = Fixture::new();
        fixture.dirs().prepare_sockets().unwrap();
        let runner = fixture.runner().with_ready_timeout(SHORT_READY);
        let sockets = fixture.dirs().sockets().clone();
        let link = sockets.with_file_name(LINKED_SOCKETS);
        symlink(&sockets, &link).unwrap();
        let id = HolderId::generate().unwrap();
        let socket = fixture.dirs().socket_path(&id);
        let _serving = UnixListener::bind(&socket).unwrap();
        let spelled = link.join(socket.file_name().unwrap());
        save_own_record(&fixture, id, spelled);
        thread::sleep(SHORT_READY.into_inner() * SUPERVISOR_DEADLINE_FACTOR + POLL);

        assert!(swept(&runner).is_empty());

        assert!(socket.exists(), "a live holder's socket is kept");
        forget_records(&fixture);
    }

    /// A holder whose record cannot be read may well be running, so both
    /// sweeps keep the socket its record file's name gives the id of, in the
    /// sockets' folder and in the old one, rather than make it unreachable
    /// for good.
    #[test]
    fn the_sweeps_keep_the_socket_of_an_unreadable_record() {
        let fixture = Fixture::new();
        let dirs = fixture.dirs();
        dirs.prepare_sockets().unwrap();
        ensure_socket_folder(dirs.legacy_sockets()).unwrap();
        let runner = fixture.runner().with_ready_timeout(SHORT_READY);
        let id = HolderId::generate().unwrap();
        let socket = dirs.socket_path(&id);
        drop(UnixListener::bind(&socket).unwrap());
        let legacy_id = HolderId::generate().unwrap();
        let legacy = dirs
            .legacy_sockets()
            .join(legacy_id.as_ref())
            .with_extension(SOCKET_EXTENSION);
        drop(UnixListener::bind(&legacy).unwrap());
        fs::create_dir_all(dirs.records()).unwrap();
        for unreadable in [&id, &legacy_id] {
            fs::write(
                dirs.records()
                    .join(unreadable.as_ref())
                    .with_extension(RECORD_EXTENSION),
                CORRUPT_SPEC,
            )
            .unwrap();
        }
        thread::sleep(SHORT_READY.into_inner() * SUPERVISOR_DEADLINE_FACTOR + POLL);

        assert!(swept(&runner).is_empty());
        assert!(legacy_swept(&runner).is_empty());

        assert!(
            socket.exists(),
            "the unreadable record's socket was removed"
        );
        assert!(
            legacy.exists(),
            "the unreadable record's old socket was removed"
        );
    }

    /// A holder an earlier muster launched with its socket in the state
    /// directory's old `holders/sock` folder is still adopted by a runner
    /// whose sockets live elsewhere, since its record names the socket's
    /// full path.
    #[test]
    fn a_holder_whose_socket_is_in_the_old_state_folder_is_adopted() {
        let fixture = Fixture::new();
        let dirs = fixture.dirs();
        let old = HolderDirs::builder()
            .sockets(dirs.legacy_sockets().clone())
            .legacy_sockets(dirs.legacy_sockets().clone())
            .records(dirs.records().clone())
            .logs(dirs.logs().clone())
            .build();
        assert_ne!(old.sockets(), dirs.sockets());
        let pid = launch_and_leave(
            &fixture,
            fixture.runner_over(
                old.clone(),
                Box::new(ThreadLauncher::new(Arc::clone(fixture.holders()))),
            ),
        );
        let runner = fixture.runner().with_ready_timeout(SHORT_READY);

        assert_adopted_and_kept(&runner, pid, old.sockets());
    }

    /// A sockets' folder that is a link, as one planted in `/tmp` would be,
    /// is never entered: what it leads to is left as it is, and the sweep
    /// reports the folder.
    #[test]
    fn the_sweep_never_enters_a_planted_sockets_folder() {
        let fixture = Fixture::new();
        let dirs = fixture.dirs();
        let elsewhere = fixture.scratch().path().join(PLANTED_TARGET);
        fs::create_dir(&elsewhere).unwrap();
        fs::set_permissions(&elsewhere, Permissions::from_mode(PRIVATE_DIR_MODE)).unwrap();
        let root = dirs.sockets().parent().unwrap();
        fs::create_dir(root).unwrap();
        fs::set_permissions(root, Permissions::from_mode(PRIVATE_DIR_MODE)).unwrap();
        symlink(&elsewhere, dirs.sockets()).unwrap();
        let id = HolderId::generate().unwrap();
        let stale = elsewhere.join(dirs.socket_path(&id).file_name().unwrap());
        drop(UnixListener::bind(&stale).unwrap());
        let spec = error_path_beside(&stale);
        fs::write(&spec, b"x").unwrap();
        let runner = fixture.runner().with_ready_timeout(SHORT_READY);
        thread::sleep(SHORT_READY.into_inner() * SUPERVISOR_DEADLINE_FACTOR + POLL);

        let failures = swept(&runner);

        assert!(
            stale.exists() && spec.exists(),
            "the planted folder was entered"
        );
        assert_eq!(
            failures
                .iter()
                .map(|failure| (failure.target(), failure.path().clone()))
                .collect::<Vec<_>>(),
            vec![(SweepTarget::Folder, dirs.sockets().clone())]
        );
        assert!(matches!(
            failures[0].error(),
            HolderError::InsecureSocketDir { .. }
        ));
    }

    /// Launches a ticking child through `runner`, then drops its handle and
    /// the runner, leaving the holder running as a stopped service would.
    /// Returns the child's pid.
    fn launch_and_leave(fixture: &Fixture, runner: HolderRunner) -> Pid {
        let (sink, mut watch) = Watch::new();
        let mut handle = runner
            .spawn(
                fixture.request("while :; do echo tick; sleep 0.05; done"),
                sink,
            )
            .unwrap();
        watch.connected(&mut handle);
        handle.process_id().unwrap()
    }

    /// Asserts that `runner`'s sweep keeps the one live holder's socket in
    /// `folder` once it has settled, and that the runner adopts that holder
    /// supervising `pid`, then kills the child.
    fn assert_adopted_and_kept(runner: &HolderRunner, pid: Pid, folder: &Path) {
        thread::sleep(SHORT_READY.into_inner() * SUPERVISOR_DEADLINE_FACTOR + POLL);

        assert!(swept(runner).is_empty());
        let records = runner.scan_records(None).live();

        assert_eq!(records.len(), 1);
        assert!(
            records[0].socket().starts_with(folder),
            "{}",
            records[0].socket().display()
        );
        assert!(records[0].socket().exists(), "the sweep kept its socket");
        let (sink, mut watch) = Watch::new();
        let mut adopted = runner.adopt(&records[0], None, sink).unwrap();
        watch.connected(&mut adopted);
        assert_eq!(adopted.process_id(), Some(pid));
        adopted.kill().unwrap();
        watch.until("the child's exit", |watch| watch.exit().is_some());
    }

    /// The sweep removes only sockets: a regular file named like one is
    /// left as it is.
    #[test]
    fn the_sweep_leaves_a_file_that_is_not_a_socket() {
        let fixture = Fixture::new();
        fixture.dirs().prepare_sockets().unwrap();
        let runner = fixture.runner().with_ready_timeout(SHORT_READY);
        let named = fixture.dirs().socket_path(&HolderId::generate().unwrap());
        fs::write(&named, b"").unwrap();
        thread::sleep(SHORT_READY.into_inner() * SUPERVISOR_DEADLINE_FACTOR + POLL);

        assert!(swept(&runner).is_empty());

        assert!(named.exists(), "a file that is not a socket was removed");
    }

    /// Saves a record of a holder that is the test process itself, known by
    /// its start token, whose child has exited and whose socket is gone, as
    /// a holder binding its socket again after something removed it is.
    fn save_rebinding_holder_record(fixture: &Fixture) -> HolderRecord {
        let dead = save_dead_holder_record(fixture);
        let record = HolderRecord::builder()
            .socket(dead.socket().clone())
            .log(dead.log().clone())
            .id(dead.id().clone())
            .project(fixture.root())
            .process(dead.process().clone())
            .command(dead.command().clone())
            .stop(test_stop())
            .holder_pid(own_pid())
            .holder_start_token(own_start_token())
            .child_pid(dead.child_pid())
            .child_start_token(None)
            .child_started_at(SystemTime::now())
            .build();
        record.save(fixture.dirs().records()).unwrap();
        record
    }

    /// A holder that is still the process its record names keeps its record
    /// while its socket is gone and its child has exited, as while it binds
    /// its socket again, so a later look still finds it to collect.
    #[test]
    fn a_live_holder_without_its_socket_keeps_its_record() {
        let fixture = Fixture::new();
        let record = save_rebinding_holder_record(&fixture);
        assert!(!record.socket().exists());

        let live = fixture.runner().scan_records(None).live();

        assert_eq!(live, vec![record.clone()]);
        assert_eq!(fixture.record_files(), vec![record]);
        forget_records(&fixture);
    }

    /// The record of a dead holder that left its child's exit is kept for
    /// adoption, and adopting it hands that exit to the sink as a waiting
    /// holder would, without reaching for its socket; acknowledging the exit
    /// removes the record and the exit with it.
    #[test]
    fn a_stranded_exit_is_adopted_and_handed_over_like_a_collected_one() {
        let fixture = Fixture::new();
        let stranded = save_dead_holder_record(&fixture);
        strand(&fixture, &stranded);
        let gone = save_dead_holder_record(&fixture);
        let runner = fixture.runner();

        let scan = runner.scan_records(None);

        assert_eq!(scan.live(), vec![stranded.clone()]);
        assert!(!scan.serving_ids().contains(stranded.id()));
        assert!(scan.is_stranded(&stranded));
        assert!(
            !fixture.record_files().contains(&gone),
            "a dead record goes"
        );
        let (sender, outputs) = unbounded();
        let mut handle = runner
            .adopt(&stranded, None, Box::new(ChannelSink::new(sender)))
            .unwrap();
        assert_eq!(
            outputs.recv_timeout(WAIT).unwrap(),
            ProcessOutput::Exited(ExitOutcome::FAILED_UNKNOWN)
        );
        assert_eq!(handle.started_at(), Some(stranded.child_started_at()));
        assert_eq!(handle.run_start(), Some(OutputCursor::new(LEFT_RUN_START)));
        assert_eq!(
            handle.process_id(),
            None,
            "a gone child's pid may name another process now"
        );
        handle.acknowledge_exit().unwrap();
        assert!(fixture.record_files().is_empty());
        assert_eq!(stranded.left_exit(fixture.dirs().records()), None);
    }

    /// Launches a child that waits for input through `runner`, then lets go
    /// of its handle, leaving the holder running unsupervised as a stopped
    /// service would. Returns the holder's record.
    fn launch_and_leave_reading(fixture: &Fixture, runner: &HolderRunner) -> HolderRecord {
        let (sink, mut watch) = Watch::new();
        let mut handle = runner.spawn(fixture.request("read x"), sink).unwrap();
        watch.connected(&mut handle);
        let record = runner.locate(handle.process_id().unwrap()).unwrap();
        drop(handle);
        record
    }

    /// Leaves a failed exit beside `record`, as a holder that could not be
    /// reached does.
    fn strand(fixture: &Fixture, record: &HolderRecord) {
        let left = LeftExit::builder()
            .outcome(ExitOutcome::FAILED_UNKNOWN)
            .run_start(OutputCursor::new(LEFT_RUN_START))
            .build();
        record
            .save_stranded(fixture.dirs().records(), &left)
            .unwrap();
    }

    /// An exit left beside the record of a holder still running as itself
    /// by its start token never overrides the holder: the scan keeps it as
    /// a serving holder, and adopting it reaches for its socket rather than
    /// report the exit or remove the record.
    #[test]
    fn an_exit_beside_a_running_holder_never_overrides_it() {
        let fixture = Fixture::new();
        let record = save_rebinding_holder_record(&fixture);
        strand(&fixture, &record);
        let runner = fixture.runner();

        let scan = runner.scan_records(None);

        assert!(scan.serving_ids().contains(record.id()));
        assert!(!scan.is_stranded(&record));
        let (sender, outputs) = unbounded();
        assert!(
            runner
                .adopt(&record, None, Box::new(ChannelSink::new(sender)))
                .is_err()
        );
        assert!(outputs.try_recv().is_err(), "no exit is reported");
        assert!(runner.take_stranded(&record).is_none());
        assert_eq!(fixture.record_files(), vec![record]);
        forget_records(&fixture);
    }

    /// An adoption reaches a holder that binds its socket again shortly
    /// after the adoption began, rather than fail at once on the missing
    /// socket.
    #[test]
    fn an_adoption_waits_out_a_holder_binding_its_socket_again() {
        let fixture = Fixture::new();
        let runner = fixture.runner();
        let record = launch_and_leave_reading(&fixture, &runner);
        fs::remove_file(record.socket()).unwrap();

        let (sink, mut watch) = Watch::new();
        let mut adopted = runner
            .adopt(&record, None, sink)
            .expect("the adoption waits for the socket to come back");
        watch.connected(&mut adopted);

        adopted.kill().unwrap();
        watch.until("the child's exit", |watch| watch.exit().is_some());
    }

    /// A reconnect's look for the holder of a child finds the holder that
    /// still runs as itself even while its socket is gone, so the reconnect
    /// adopts it rather than settle the run as lost.
    #[test]
    fn a_holder_without_its_socket_is_still_located() {
        let fixture = Fixture::new();
        let record = save_rebinding_holder_record(&fixture);

        assert_eq!(fixture.runner().locate(record.child_pid()), Some(record));
        forget_records(&fixture);
    }

    /// Collecting the exit of a holder that is gone and left it beside its
    /// record takes that exit and removes the record, without dialling a
    /// socket nothing serves, so nothing waits on it or reports it again.
    #[test]
    fn collecting_a_stranded_holder_takes_its_exit_without_dialling() {
        let fixture = Fixture::new();
        let stranded = save_dead_holder_record(&fixture);
        strand(&fixture, &stranded);
        let runner = fixture.runner();
        let started = Instant::now();

        let collected = HolderAdopter::collect(&runner, &stranded, true);

        assert_eq!(collected, Some(ExitOutcome::FAILED_UNKNOWN));
        assert!(
            started.elapsed() < ABANDON_TIMEOUT,
            "no socket was waited on"
        );
        assert!(fixture.record_files().is_empty());
        assert_eq!(stranded.left_exit(fixture.dirs().records()), None);
    }

    /// A holder reached and collected has its record removed with it, so no
    /// exit of its run outlives the collect.
    #[test]
    fn a_collected_holder_leaves_no_record() {
        let fixture = Fixture::new();
        let runner = fixture.runner();
        let record = launch_and_leave_reading(&fixture, &runner);

        let collected = HolderAdopter::collect(&runner, &record, true);

        assert!(collected.is_some_and(|outcome| outcome.is_failure()));
        assert!(!fixture.record_files().contains(&record));
    }

    /// A record made in another boot names a holder and a child that are
    /// gone, whatever runs now with their pids and start tokens.
    #[test]
    fn a_record_from_another_boot_names_nothing_running() {
        let fixture = Fixture::new();
        let record = save_rebinding_holder_record(&fixture);
        let Some(boot) = LocalProcessIdentity::boot_id() else {
            forget_records(&fixture);
            return;
        };
        let earlier = BootId::try_new(format!("{boot}{EARLIER_BOOT_SUFFIX}")).unwrap();
        let from_earlier = HolderRecord::builder()
            .socket(record.socket().clone())
            .log(record.log().clone())
            .id(record.id().clone())
            .project(fixture.root())
            .process(record.process().clone())
            .command(record.command().clone())
            .stop(test_stop())
            .holder_pid(own_pid())
            .holder_start_token(own_start_token())
            .boot_id(Some(earlier))
            .child_pid(own_pid())
            .child_start_token(own_start_token())
            .child_started_at(SystemTime::now())
            .build();

        assert_eq!(from_earlier.alive_by_token(), Some(false));
        assert!(!child_runs(&from_earlier));
        assert_eq!(record.alive_by_token(), Some(true));
        forget_records(&fixture);
    }

    /// The scan reads the records once, prunes the dead ones and keeps what
    /// it found for every pass: a record removed or added after it was taken
    /// changes nothing it says.
    #[test]
    fn the_passes_share_one_scan() {
        let fixture = Fixture::new();
        let record = save_rebinding_holder_record(&fixture);
        let runner = fixture.runner();
        let scan = runner.scan_records(None);

        record.remove(fixture.dirs().records()).unwrap();
        let later = save_dead_holder_record(&fixture);

        assert_eq!(scan.live(), vec![record.clone()]);
        let serving = scan.serving_ids();
        assert!(serving.contains(record.id()) && !serving.contains(later.id()));
        forget_records(&fixture);
    }

    /// The scan of one project reads and prunes only that project's records,
    /// leaving every other project's as they are.
    #[test]
    fn a_scan_of_one_project_leaves_the_others() {
        let fixture = Fixture::new();
        let dead = save_dead_holder_record(&fixture);
        let elsewhere = ProjectRoot::try_new(fixture.scratch().path().join(OTHER_PROJECT)).unwrap();

        let scan = fixture.runner().scan_records(Some(&elsewhere));

        assert!(scan.live().is_empty());
        assert_eq!(fixture.record_files(), vec![dead]);
        forget_records(&fixture);
    }

    /// Serving records come first in the scan, then the records of dead
    /// holders whose child runs on, then the exits left, whatever order the
    /// folder lists them in, so a live copy is adopted before an exit left
    /// of the same process.
    #[test]
    fn the_scan_puts_serving_records_before_stranded_ones() {
        let fixture = Fixture::new();
        for _ in 0..STALE_RECORDS {
            let stranded = save_dead_holder_record(&fixture);
            strand(&fixture, &stranded);
        }
        let live = save_rebinding_holder_record(&fixture);

        let scan = fixture.runner().scan_records(None);

        assert_eq!(scan.live().first(), Some(&live));
        assert_eq!(scan.stranded().len(), STALE_RECORDS);
        forget_records(&fixture);
    }

    /// The old sweep has nothing to report when the old sockets folder was
    /// never made, even when the `holders` folder that would hold it is open
    /// to others, so a service start does not warn about it every time.
    #[test]
    fn the_old_sweep_is_silent_without_the_old_folder() {
        let fixture = Fixture::new();
        let holders = fixture
            .dirs()
            .legacy_sockets()
            .parent()
            .unwrap()
            .to_path_buf();
        fs::create_dir_all(&holders).unwrap();
        fs::set_permissions(&holders, Permissions::from_mode(SHARED_DIR_MODE)).unwrap();

        let failures = legacy_swept(&fixture.runner());

        assert!(failures.is_empty(), "{failures:?}");
    }

    /// The old sweep goes by the rule adoption goes by: a socket whose
    /// record names a live pid that no longer accepts on it, as a reused
    /// pid's does, belongs to a holder that ended and is removed, never kept
    /// for good.
    #[test]
    fn the_old_sweep_removes_a_socket_whose_pid_was_reused() {
        let fixture = Fixture::new();
        let legacy = fixture.dirs().legacy_sockets().clone();
        ensure_socket_folder(&legacy).unwrap();
        let id = HolderId::generate().unwrap();
        let socket = legacy.join(id.as_ref()).with_extension(SOCKET_EXTENSION);
        let _refusing = UnixDatagram::bind(&socket).unwrap();
        save_own_record(&fixture, id, socket.clone());
        let runner = fixture.runner();

        assert!(legacy_swept(&runner).is_empty());

        assert!(!socket.exists(), "a reused pid's socket was kept");
        forget_records(&fixture);
    }

    /// The sweep of the sockets' folder `runner` makes at service start,
    /// over its records as it scans them.
    fn swept(runner: &HolderRunner) -> Vec<SweepFailure> {
        runner.sweep_sockets(&runner.scan_records(None))
    }

    /// The sweep of the old sockets folder `runner` makes at service start,
    /// over its records as it scans them.
    fn legacy_swept(runner: &HolderRunner) -> Vec<SweepFailure> {
        runner.sweep_legacy_sockets(&runner.scan_records(None))
    }

    /// The folder an earlier muster bound holder sockets in loses every
    /// handover file, a running holder's spec and error file included, since
    /// a spec is read once at launch and an error file only by the launch,
    /// and the sockets of holders no longer running; it keeps a running
    /// holder's socket, and is removed once that holder is gone too.
    #[test]
    fn the_old_sockets_folder_is_swept_then_removed() {
        let fixture = Fixture::new();
        let legacy = fixture.dirs().legacy_sockets().clone();
        ensure_socket_folder(&legacy).unwrap();
        let in_legacy = |id: &HolderId| legacy.join(id.as_ref()).with_extension(SOCKET_EXTENSION);
        let orphan = in_legacy(&HolderId::generate().unwrap());
        drop(UnixListener::bind(&orphan).unwrap());
        let unnamed_spec = error_path_beside(&in_legacy(&HolderId::generate().unwrap()));
        fs::write(&unnamed_spec, b"env").unwrap();
        let id = HolderId::generate().unwrap();
        let served = in_legacy(&id);
        let serving = UnixListener::bind(&served).unwrap();
        let record = save_own_record(&fixture, id, served.clone());
        let named_spec = served.with_extension(SPEC_EXTENSION);
        fs::write(&named_spec, b"env").unwrap();
        let named_error = error_path_beside(&named_spec);
        fs::write(&named_error, b"").unwrap();
        let runner = fixture.runner();

        assert!(legacy_swept(&runner).is_empty());

        assert!(!orphan.exists() && !unnamed_spec.exists());
        assert!(
            !named_spec.exists() && !named_error.exists(),
            "a running holder's handover files linger"
        );
        assert!(served.exists(), "a running holder's socket is kept");
        drop(serving);
        record.remove(fixture.dirs().records()).unwrap();
        assert!(legacy_swept(&runner).is_empty());
        assert!(!legacy.exists(), "the emptied folder is removed");
    }

    /// Saves a record whose holder process has exited.
    fn save_dead_holder_record(fixture: &Fixture) -> HolderRecord {
        let mut exited = process::Command::new("true").spawn().unwrap();
        let dead_pid = Pid::try_new(exited.id()).unwrap();
        exited.wait().unwrap();
        let id = HolderId::generate().unwrap();
        let name = ProcessName::try_new("dev server").unwrap();
        let record = HolderRecord::builder()
            .socket(fixture.dirs().socket_path(&id))
            .log(fixture.dirs().process_log(&fixture.root(), &name))
            .id(id)
            .project(fixture.root())
            .process(name)
            .command(CommandLine::try_new("true").unwrap())
            .stop(test_stop())
            .holder_pid(dead_pid)
            .child_pid(dead_pid)
            .child_start_token(None)
            .child_started_at(SystemTime::now())
            .build();
        record.save(fixture.dirs().records()).unwrap();
        record
    }

    #[test]
    fn a_holder_the_runner_gave_up_on_kills_its_child_and_leaves_no_record() {
        let fixture = Fixture::new();
        let runner = fixture
            .runner_with(Box::new(ThreadLauncher::delayed(
                Arc::clone(fixture.holders()),
                SLOW_START,
            )))
            .with_ready_timeout(SHORT_READY);
        let pid_file = fixture.scratch().path().join(PID_FILE);
        let command = format!(
            "echo $$ > {}; while :; do sleep 0.05; done",
            pid_file.display()
        );

        let (sink, _watch) = Watch::new();
        assert!(runner.spawn(fixture.request(&command), sink).is_err());
        wait_until("the abandoned holder to stop", || {
            fixture
                .holders()
                .lock()
                .unwrap()
                .iter()
                .all(JoinHandle::is_finished)
        });
        assert!(fixture.record_files().is_empty(), "no record survives");
        // The kill can land before the child writes its pid; a holder that
        // finished has then already seen it exit. An absent pid file must
        // stay absent, or a surviving child wrote it late.
        let reported_pid = || {
            fs::read_to_string(&pid_file)
                .ok()
                .and_then(|pid| pid.trim().parse::<u32>().ok())
        };
        let pid = reported_pid().or_else(|| {
            thread::sleep(PID_FILE_GRACE);
            reported_pid()
        });
        if let Some(pid) = pid {
            let child = Pid::try_new(pid).unwrap();
            wait_until("the abandoned child to die", || {
                !SignalTarget::Process(child).is_alive()
            });
        }
    }

    #[test]
    fn abandoning_a_holder_kills_its_child_and_lets_it_finish() {
        let fixture = Fixture::new();
        let runner = fixture.runner();
        let (sink, _watch) = Watch::new();
        let handle = runner
            .spawn(fixture.request("while :; do sleep 0.05; done"), sink)
            .unwrap();
        let record = runner.locate(handle.process_id().unwrap()).unwrap();
        drop(handle);

        abandon(record.socket());
        wait_until("the holder to finish", || {
            fixture
                .holders()
                .lock()
                .unwrap()
                .iter()
                .all(JoinHandle::is_finished)
        });
        assert!(fixture.record_files().is_empty());
        wait_until("the child to die", || {
            !SignalTarget::Process(record.child_pid()).is_alive()
        });
    }

    /// Collecting a holder's exit never signals its child: the holder keeps
    /// it running until something else ends it, then hands over the exit and
    /// finishes.
    #[test]
    fn collecting_a_holder_leaves_its_child_running_until_it_ends() {
        let fixture = Fixture::new();
        let runner = fixture.runner();
        let (sink, _watch) = Watch::new();
        let handle = runner
            .spawn(fixture.request("while :; do sleep 0.05; done"), sink)
            .unwrap();
        let record = runner.locate(handle.process_id().unwrap()).unwrap();
        drop(handle);
        let socket = record.socket().clone();

        let collector = thread::spawn(move || finish(&socket, false));
        thread::sleep(ABANDON_TIMEOUT / 4);
        let child = SignalTarget::Process(record.child_pid());
        assert!(child.is_alive(), "collecting signalled the child");
        child.send(Signal::KILL).unwrap();
        collector.join().unwrap();

        wait_until("the holder to finish", || {
            fixture
                .holders()
                .lock()
                .unwrap()
                .iter()
                .all(JoinHandle::is_finished)
        });
        assert!(fixture.record_files().is_empty());
    }

    /// Abandoning a holder's handle through the port stops its child and lets
    /// the holder finish, without the caller waiting on either.
    #[test]
    fn abandoning_a_holder_handle_stops_its_child_and_lets_it_finish() {
        let fixture = Fixture::new();
        let runner = fixture.runner();
        let (sink, _watch) = Watch::new();
        let handle = runner
            .spawn(fixture.request("while :; do sleep 0.05; done"), sink)
            .unwrap();
        let record = runner.locate(handle.process_id().unwrap()).unwrap();
        assert_eq!(
            format!("{:?}", handle.screen_source()),
            format!(
                "{:?}",
                ScreenSource::Served(Arc::new(ServedScreen::new(record.socket().clone())))
            )
        );

        handle.abandon();
        wait_until("the holder to finish", || {
            fixture
                .holders()
                .lock()
                .unwrap()
                .iter()
                .all(JoinHandle::is_finished)
        });
        assert!(fixture.record_files().is_empty());
        wait_until("the child to die", || {
            !SignalTarget::Process(record.child_pid()).is_alive()
        });
    }

    #[test]
    fn a_holder_that_fails_before_serving_reports_why() {
        let fixture = Fixture::new();
        let runner = fixture.runner_with(fixture.spec_launcher(Some(Sabotage::CorruptSpec)));

        let error = expect_failed_start(&fixture, &runner, fixture.request("true"));
        assert!(error.contains("json"), "the holder's own error: {error}");
    }

    #[test]
    fn a_holder_whose_log_cannot_open_reports_why_without_serving() {
        let fixture = Fixture::new();
        let blocker = fixture.scratch().path().join(NOT_A_DIRECTORY);
        fs::write(&blocker, b"").unwrap();
        let dirs = HolderDirs::scratch(fixture.scratch().path(), blocker.join("logs"));
        let runner = fixture.runner_over(dirs, fixture.spec_launcher(None));

        let error = expect_failed_start(&fixture, &runner, fixture.request("true"));
        assert!(error.contains("i/o"), "the holder's own error: {error}");
    }

    #[test]
    fn a_holder_whose_child_cannot_spawn_reports_why_without_serving() {
        let fixture = Fixture::new();
        let workdir = fixture.scratch().path().join("work");
        fs::create_dir(&workdir).unwrap();
        let runner =
            fixture.runner_with(fixture.spec_launcher(Some(Sabotage::RemoveDir(workdir.clone()))));
        let request = test_request(
            "dev server",
            "true",
            Some(fixture.scratch().path().to_path_buf()),
            Some(workdir),
        );

        let error = expect_failed_start(&fixture, &runner, request);
        assert!(
            error.contains("working directory"),
            "the holder's own error: {error}"
        );
    }

    /// Spawns `request`, which its holder must fail to start, and returns the
    /// error, checking it carries the holder's report, arrives before the
    /// ready timeout and leaves no handover files or record behind.
    fn expect_failed_start(
        fixture: &Fixture,
        runner: &HolderRunner,
        request: SpawnRequest,
    ) -> String {
        let started = Instant::now();
        let (sink, _watch) = Watch::new();
        let error = match runner.spawn(request, sink) {
            Err(error) => error.to_string(),
            Ok(_) => panic!("a sabotaged holder cannot serve"),
        };
        assert!(error.contains("failed to start"), "error: {error}");
        assert!(
            started.elapsed() < HOLDER_READY_TIMEOUT.into_inner(),
            "the report ends the wait"
        );
        let leftovers = fs::read_dir(fixture.dirs().sockets()).unwrap().count();
        assert_eq!(leftovers, 0, "no socket, spec or error file is left");
        assert!(fixture.record_files().is_empty(), "no record is left");
        error
    }

    #[test]
    fn abandoning_a_holder_still_starting_reaches_it_once_it_serves() {
        let fixture = Fixture::new();
        fixture.dirs().prepare_sockets().unwrap();
        let config = fixture.config("while :; do sleep 0.05; done", LONG_SUPERVISOR_DEADLINE);
        let socket = config.socket().clone();
        ThreadLauncher::delayed(Arc::clone(fixture.holders()), STARTING_DELAY)
            .launch(config)
            .unwrap();

        abandon(&socket);
        wait_until("the abandoned holder to finish", || {
            fixture
                .holders()
                .lock()
                .unwrap()
                .iter()
                .all(JoinHandle::is_finished)
        });
        assert!(fixture.record_files().is_empty());
    }

    #[test]
    fn terminate_honours_signal_and_grace_then_kills() {
        let fixture = Fixture::new();
        let runner = fixture.runner();

        let (sink, mut watch) = Watch::new();
        let mut handle = runner
            .spawn(
                fixture.request(
                    "trap 'printf got-int; exit 0' INT; printf ready; while :; do sleep 0.05; done",
                ),
                sink,
            )
            .unwrap();
        watch.connected(&mut handle);
        watch.until("the trap to be set", |watch| watch.text().contains("ready"));
        let started = Instant::now();
        handle.terminate(StopSignal::Interrupt, WAIT).unwrap();
        watch.until("the graceful exit", |watch| watch.exit().is_some());
        assert!(watch.text().contains("got-int"), "the signal was delivered");
        assert_eq!(watch.exit(), Some(ExitOutcome::Succeeded));
        assert!(started.elapsed() < WAIT, "a graceful exit needs no kill");

        let (sink, mut watch) = Watch::new();
        let mut handle = runner
            .spawn(
                fixture.request("trap '' TERM; printf ready; while :; do sleep 0.05; done"),
                sink,
            )
            .unwrap();
        watch.connected(&mut handle);
        watch.until("the trap to be set", |watch| watch.text().contains("ready"));
        let started = Instant::now();
        handle.terminate(StopSignal::Terminate, STOP_GRACE).unwrap();
        thread::sleep(STOP_GRACE / 2);
        watch.drain();
        assert!(
            watch.exit().is_none(),
            "the child outlives the ignored signal within its grace"
        );
        watch.until("the forced exit", |watch| watch.exit().is_some());
        assert!(
            matches!(
                watch.exit(),
                Some(ExitOutcome::Failed(ExitCause::Signal(_)))
            ),
            "the forced stop ends the child by a signal: {:?}",
            watch.exit()
        );
        assert!(
            started.elapsed() >= STOP_GRACE,
            "the kill waits out the grace"
        );
    }

    /// Sweeping removes the spec and error files beside holder sockets and
    /// nothing else; a missing socket directory holds nothing to sweep.
    #[test]
    fn sweeping_removes_only_handover_files() {
        let fixture = Fixture::new();
        let runner = fixture.runner();
        assert!(swept(&runner).is_empty());
        fixture.dirs().prepare_sockets().unwrap();
        let id = HolderId::generate().unwrap();
        let socket = fixture.dirs().socket_path(&id);
        let spec = fixture.config("true", LONG_SUPERVISOR_DEADLINE).spec_path();
        let error = error_path_beside(&spec);
        for path in [&socket, &spec, &error] {
            fs::write(path, b"x").unwrap();
        }
        let stuck = fixture.dirs().sockets().join(STUCK_HANDOVER);
        fs::create_dir(&stuck).unwrap();

        let failures = swept(&runner);

        assert!(socket.exists());
        assert!(
            !spec.exists() && !error.exists(),
            "the sweep goes on past a failure"
        );
        assert_eq!(
            failures
                .iter()
                .map(|failure| (failure.target(), failure.path().clone()))
                .collect::<Vec<_>>(),
            vec![(SweepTarget::HandoverFile, stuck)]
        );
    }

    /// A record file that cannot be parsed is reported, never adopted.
    #[test]
    fn an_unreadable_record_is_reported() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.dirs().records()).unwrap();
        let corrupt = fixture.dirs().records().join("corrupt.json");
        fs::write(&corrupt, CORRUPT_SPEC).unwrap();

        let runner = fixture.runner();

        let scan = runner.scan_records(None);
        assert_eq!(scan.unreadable(), &vec![corrupt]);
        assert!(scan.live().is_empty());
    }
}
