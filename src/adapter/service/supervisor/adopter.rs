use std::{sync::Arc, thread};

use super::{
    carried_pane::CarriedPane, channel_output_sink::ChannelOutputSink, entity::Supervisor,
    launch_purpose::LaunchPurpose, lifecycle::discard, pending_launch::PendingLaunch,
    spawn_generation::SpawnGeneration, unsupervised_child::UnsupervisedChild,
};
use crate::{
    adapter::{
        holder::{HolderRecord, HolderRunner},
        service::{NoticeLevel, ProjectKey, ServiceEvent},
    },
    domain::{
        port::{OutputSink, ProcessHandle},
        process::{ExitIntent, ProcessState},
        pty::{ExitOutcome, PtyError, SpawnHistory},
        value::PaneId,
    },
};

/// How an adoption proceeds: inline, its result already in hand, or on a
/// worker with the adopter and sink it takes there.
enum Adoption {
    /// The adoption ran on the caller's thread.
    Inline(Result<Box<dyn ProcessHandle>, PtyError>),
    /// The adoption runs on a worker.
    Detached(Arc<dyn HolderAdopter + Send + Sync>, ChannelOutputSink),
}

/// Shown when a holder record belongs to a process the config no longer has.
const ADOPT_UNKNOWN_NOTICE: &str =
    "a running process is no longer in muster.yml; it was left running unsupervised";
/// Shown when a holder record names this project's process but was recorded for
/// another project or another kind of process.
const ADOPT_MISMATCH_NOTICE: &str =
    "a running process does not match its muster.yml entry; it was left running unsupervised";
/// Shown when a holder record names a process that is already supervised.
const ADOPT_DUPLICATE_NOTICE: &str =
    "a second running copy of this process was found; it was left running unsupervised";
/// Shown when a holder record cannot be adopted for want of an adopter; an
/// outcome suffix follows.
const ADOPT_UNAVAILABLE_NOTICE: &str = "this service cannot adopt running processes";
/// Prefix for a holder that could not be reached for adoption; an outcome
/// suffix follows.
const ADOPT_FAILED_NOTICE: &str = "could not reconnect to a running process";
/// Outcome of a failed adoption whose child still runs.
const LEFT_RUNNING: &str = "; it was left running unsupervised";
/// Outcome of a failed adoption whose child is already gone.
const SINCE_EXITED: &str = "; it has since exited";
/// Why a broken connection could not be reconnected: no holder record names
/// the child pid that follows.
const NO_HOLDER_RECORD: &str = "no holder record names pid";
/// Label of a record's child pid in a notice.
const CHILD_PID_LABEL: &str = "child pid";
/// Label of a record's holder pid in a notice.
const HOLDER_PID_LABEL: &str = "holder pid";

/// `notice` naming the child and holder pids of `record`, so the user can find
/// the process left running.
fn with_pids(notice: &str, record: &HolderRecord) -> String {
    format!(
        "{notice} ({CHILD_PID_LABEL} {}, {HOLDER_PID_LABEL} {})",
        record.child_pid(),
        record.holder_pid()
    )
}

/// The outcome suffix of a failed adoption whose child is `running` or not.
fn outcome(running: bool) -> &'static str {
    if running { LEFT_RUNNING } else { SINCE_EXITED }
}

/// Takes over a holder a previous service left running, so its process is
/// supervised again without being restarted, and finds the holder of a running
/// process.
pub trait HolderAdopter {
    /// The record of the holder running the child with `child_pid`, if a
    /// holder runs it.
    fn locate(&self, child_pid: u32) -> Option<HolderRecord>;

    /// Becomes the supervisor of the holder `record` describes, forwarding its
    /// screen, output and exit to `sink`.
    ///
    /// # Errors
    /// Returns a [`PtyError`] when the holder cannot be reached or sends no
    /// screen.
    fn adopt(
        &self,
        record: &HolderRecord,
        sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError>;

    /// This adopter, shareable with a worker thread, when an adoption may
    /// block long enough that the supervisor must not wait for it on its
    /// loop. Defaults to none: adoptions run inline.
    fn detached(&self) -> Option<Arc<dyn HolderAdopter + Send + Sync>> {
        None
    }
}

impl HolderAdopter for HolderRunner {
    fn locate(&self, child_pid: u32) -> Option<HolderRecord> {
        HolderRunner::locate(self, child_pid)
    }

    fn adopt(
        &self,
        record: &HolderRecord,
        sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        HolderRunner::adopt(self, record, sink)
    }
}

impl Supervisor {
    /// Supervises the running holder `record` describes as its named process,
    /// without restarting it. The record must name this project and a process
    /// of the same kind that is not already supervised; otherwise the process
    /// is left running and a notice says so.
    pub fn adopt(&mut self, record: HolderRecord) {
        self.begin_adoption(&record, None);
    }

    /// Settles an adoption a worker finished on the loop: supervises the
    /// holder, or leaves its child unsupervised when it still runs. A result
    /// a later action superseded is never installed; its process is detached
    /// when a shutdown superseded it and stopped otherwise.
    pub fn handle_adopted(
        &mut self,
        pane: PaneId,
        generation: SpawnGeneration,
        record: &HolderRecord,
        result: Result<Box<dyn ProcessHandle>, PtyError>,
    ) {
        let queued = self.pending_session_reopens.get(&pane).cloned();
        if let Some(retired) = self.settle_adoption(pane, generation, record, result) {
            self.exit_effect(retired, queued);
        }
    }

    /// Reconnects to the holder of `pane`'s process after its output stream
    /// broke: drops the handle, then adopts the holder again, which resyncs the
    /// screen from a fresh snapshot. Everything about the pane that outlives a
    /// connection carries over: its state, exit intent, config membership and
    /// start time, and a stop in flight gets a fresh force-stop deadline. When
    /// the holder cannot be adopted and the child still runs, the process is
    /// left unsupervised, as an unadoptable holder is at startup; a child that
    /// is gone settles as a failed exit under the pane's policy. A failed
    /// reconnect always raises a notice. The adoption runs off the loop when
    /// the adopter allows it, and settles in [`Self::handle_adopted`]. Returns
    /// whether the row retired when the reconnect settled an exit here, or
    /// `None` when it did not.
    pub(super) fn reconnect(&mut self, pane: PaneId) -> Option<bool> {
        let state = self.workspace.process(pane).map(|process| *process.state());
        let target = self.panes.get_mut(&pane)?;
        let handle = target.handle.take()?;
        let carried = CarriedPane {
            intent: target.exit_intent,
            membership: target.config_membership,
            started_at: target.started_at,
            state,
            pid: handle.process_id(),
        };
        drop(handle);
        let record = carried
            .pid
            .and_then(|pid| self.adopter.as_ref()?.locate(pid));
        match record {
            Some(record) => self.begin_adoption(&record, Some((pane, carried))),
            None => {
                self.bump_generation(pane);
                self.reconnect_failed(pane, carried, None, None)
            },
        }
    }

    /// Settles a reconnect that could not supervise the holder again: the
    /// child is left unsupervised while it runs, with a stop in flight carried
    /// over, and a gone child settles as a failed exit. `record` is the
    /// holder's record when one was found and `error` why adopting it failed.
    /// Returns whether the row retired.
    fn reconnect_failed(
        &mut self,
        pane: PaneId,
        carried: CarriedPane,
        record: Option<&HolderRecord>,
        error: Option<PtyError>,
    ) -> Option<bool> {
        let start_token = record.and_then(HolderRecord::child_start_token);
        let running = carried
            .pid
            .is_some_and(|pid| self.leave_unsupervised(pane, pid, start_token));
        match (record, error) {
            (Some(record), Some(error)) => {
                let notice = format!("{ADOPT_FAILED_NOTICE}{}: {error}", outcome(running));
                self.notify(NoticeLevel::Error, Some(pane), with_pids(&notice, record));
            },
            (None, _) => {
                let pid = carried.pid.map(|pid| pid.to_string()).unwrap_or_default();
                self.notify(
                    NoticeLevel::Error,
                    Some(pane),
                    format!(
                        "{ADOPT_FAILED_NOTICE}{}: {NO_HOLDER_RECORD} {pid}",
                        outcome(running)
                    ),
                );
            },
            (Some(_), None) => {},
        }
        if running {
            self.continue_stop_unsupervised(pane, carried.intent);
            return None;
        }
        if let Some(target) = self.panes.get_mut(&pane) {
            target.exit_intent = carried.intent;
            target.config_membership = carried.membership;
        }
        self.apply_exit(pane, ExitOutcome::FAILED_UNKNOWN)
    }

    /// Carries a stop that was in flight when the connection broke over to the
    /// child now left unsupervised, so it is signalled directly rather than
    /// the user's stop being dropped.
    fn continue_stop_unsupervised(&mut self, pane: PaneId, intent: ExitIntent) {
        if intent.is_stop() {
            self.stop_unsupervised(pane, false);
        }
    }

    /// Gives a stop carried over a reconnect a fresh force-stop deadline, since
    /// the one armed before belonged to the old connection's generation.
    fn rearm_force_stop(&mut self, pane: PaneId) {
        let Some(policy) = self.stop_policy_of(pane) else {
            return;
        };
        let shutdown_generation = self.advance_shutdown_generation(pane);
        if let (Some(spawn_generation), Some(shutdown_generation)) =
            (self.generations.get(&pane).copied(), shutdown_generation)
        {
            self.schedule_force_stop(
                pane,
                spawn_generation,
                shutdown_generation,
                *policy.grace_period(),
            );
        }
    }

    /// Starts adopting the holder `record` describes, as [`Self::adopt`] does,
    /// or reconnecting `reconnecting`'s pane to it with what that pane
    /// carries. The pane is `Starting` until the adoption settles; it runs on
    /// a worker when the adopter allows it, and inline otherwise. Returns
    /// whether the row retired when a failed reconnect settled an exit here.
    fn begin_adoption(
        &mut self,
        record: &HolderRecord,
        reconnecting: Option<(PaneId, CarriedPane)>,
    ) -> Option<bool> {
        let Some(pane) = self.adoptable_pane(record) else {
            let (pane, carried) = reconnecting?;
            self.bump_generation(pane);
            return self.reconnect_failed(pane, carried, Some(record), None);
        };
        let carried = reconnecting.map(|(_, carried)| carried);
        let generation = self.bump_generation(pane);
        let project = self.project.clone();
        let events = self.events.clone();
        let sink = ChannelOutputSink::new(project.clone(), pane, generation, events.clone());
        let Some(adopter) = self.adopter.as_ref() else {
            let running =
                self.leave_unsupervised(pane, record.child_pid(), record.child_start_token());
            let notice = format!("{ADOPT_UNAVAILABLE_NOTICE}{}", outcome(running));
            self.notify(NoticeLevel::Error, Some(pane), with_pids(&notice, record));
            return None;
        };
        let adoption = match adopter.detached() {
            Some(detached) => Adoption::Detached(detached, sink),
            None => Adoption::Inline(adopter.adopt(record, Box::new(sink))),
        };
        let prior_state = self
            .workspace
            .process(pane)
            .map_or(ProcessState::Pending, |process| *process.state());
        self.launches.insert(
            pane,
            PendingLaunch::new(generation, prior_state, LaunchPurpose::Adopt { carried }),
        );
        self.workspace.set_state(pane, ProcessState::Starting);
        match adoption {
            Adoption::Detached(adopter, sink) => {
                let record = record.clone();
                thread::spawn(move || {
                    let result = adopter.adopt(&record, Box::new(sink));
                    let _ = events.send(ServiceEvent::Adopted {
                        project,
                        pane,
                        generation,
                        record: Box::new(record),
                        result,
                    });
                });
                None
            },
            Adoption::Inline(result) => self.settle_adoption(pane, generation, record, result),
        }
    }

    /// The pane of the process `record` runs, when the record names this
    /// project and a process of the same kind that holds nothing yet; a
    /// notice says why otherwise, and the process is left running.
    fn adoptable_pane(&self, record: &HolderRecord) -> Option<PaneId> {
        let Some(process) = self
            .workspace
            .processes()
            .iter()
            .find(|process| process.name() == record.process())
        else {
            self.notify_process(
                NoticeLevel::Warning,
                Some(record.process().clone()),
                with_pids(ADOPT_UNKNOWN_NOTICE, record),
            );
            return None;
        };
        let pane = *process.id();
        if ProjectKey::new(record.project().clone()) != self.project
            || *process.kind() != record.kind()
        {
            self.notify(
                NoticeLevel::Warning,
                Some(pane),
                with_pids(ADOPT_MISMATCH_NOTICE, record),
            );
            return None;
        }
        if self.holds_process(pane) {
            self.notify(
                NoticeLevel::Warning,
                Some(pane),
                with_pids(ADOPT_DUPLICATE_NOTICE, record),
            );
            return None;
        }
        Some(pane)
    }

    /// Settles the adoption of `pane` at `generation` with `result`: the
    /// holder is supervised with the output it sent meanwhile, a reconnect
    /// gets back what it carried, and a failure leaves a running child
    /// unsupervised. Then what was asked while it was starting applies: the
    /// membership a reconcile gave it, a pause or resume when the adoption
    /// installed a live process, and a stop, kill or restart, which a child
    /// left unsupervised receives through the unsupervised path. A superseded
    /// result is never installed: it is detached when a shutdown superseded
    /// it and stopped otherwise, and a superseded failure
    /// whose child still runs is said so. Returns whether the row retired when
    /// a failed reconnect settled an exit.
    fn settle_adoption(
        &mut self,
        pane: PaneId,
        generation: SpawnGeneration,
        record: &HolderRecord,
        result: Result<Box<dyn ProcessHandle>, PtyError>,
    ) -> Option<bool> {
        let Some(launch) = self.take_launch(pane, generation) else {
            let release = self.stale_release(pane, generation);
            match result {
                Ok(handle) => release.release(handle),
                Err(error) => self.report_superseded_failure(record, &error),
            }
            return None;
        };
        let PendingLaunch {
            prior_state,
            purpose,
            early,
            intent,
            suspend,
            membership,
            ..
        } = launch;
        let LaunchPurpose::Adopt { carried, .. } = purpose else {
            if let Ok(handle) = result {
                discard(handle);
            }
            return None;
        };
        let carried = carried.map(|carried| CarriedPane {
            membership: membership.unwrap_or(carried.membership),
            ..carried
        });
        let handle = match result {
            Ok(handle) => handle,
            Err(error) => {
                self.workspace.set_state(pane, prior_state);
                if let Some(carried) = carried {
                    let retired = self.reconnect_failed(pane, carried, Some(record), Some(error));
                    if retired != Some(true) {
                        self.apply_pending(pane, membership, None, intent);
                    }
                    return retired;
                }
                let running =
                    self.leave_unsupervised(pane, record.child_pid(), record.child_start_token());
                let notice = format!("{ADOPT_FAILED_NOTICE}{}: {error}", outcome(running));
                self.notify(NoticeLevel::Error, Some(pane), with_pids(&notice, record));
                self.apply_pending(pane, membership, None, intent);
                return None;
            },
        };
        self.install_pane(pane, handle);
        if carried.is_none() {
            self.take_history(pane, record.history());
        }
        if let Some(carried) = carried {
            if let Some(target) = self.panes.get_mut(&pane) {
                target.exit_intent = carried.intent;
                target.config_membership = carried.membership;
                target.started_at = carried.started_at;
            }
            if let Some(state) = carried.state {
                self.workspace.set_state(pane, state);
            }
            if carried.intent.awaits_force_stop() {
                self.rearm_force_stop(pane);
            }
        }
        self.replay(pane, generation, early);
        let suspend = suspend.filter(|_| self.runs_launch(pane, generation));
        self.apply_pending(pane, membership, suspend, intent);
        None
    }

    /// Takes over the restart count and last exit a holder record carries
    /// from the service that spawned its child, so `ps` keeps showing them.
    fn take_history(&mut self, pane: PaneId, history: &SpawnHistory) {
        if history.restarts() > 0 {
            self.restarts.insert(pane, history.restarts());
        }
        if let Some(last_exit) = history.last_exit() {
            self.last_exits.insert(pane, last_exit.clone());
        }
    }

    /// Reports an adoption that failed after a later action superseded it,
    /// when its child still runs, so a child is never dropped silently.
    fn report_superseded_failure(&self, record: &HolderRecord, error: &PtyError) {
        let running =
            UnsupervisedChild::observe_expected(record.child_pid(), record.child_start_token())
                .is_some();
        if running {
            self.notify_process(
                NoticeLevel::Error,
                Some(record.process().clone()),
                with_pids(
                    &format!("{ADOPT_FAILED_NOTICE}{LEFT_RUNNING}: {error}"),
                    record,
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        path::PathBuf,
        rc::Rc,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        time::{Duration, Instant, SystemTime},
    };

    use super::*;
    use crate::{
        adapter::{
            holder::HolderId,
            service::{
                ServiceEvent,
                supervisor::{
                    ConfigMembership, OutputEffect, SpawnGeneration, lifecycle::FORCE_STOP_NOTICE,
                    testing::*, unsupervised::UNSUPERVISED_ROW_REMOVED,
                    unsupervised_child::UnsupervisedChild,
                },
            },
        },
        domain::{
            agent_session::AgentProcessStartToken,
            config::{ProcessSpec, WorkspaceConfig},
            port::ProcessRunner,
            process::{
                AgentTool, Process, ProcessKind, ProcessState, RestartPolicy, StopPolicy,
                StopSignal,
            },
            pty::{ExitOutcome, ProcessOutput, PtySize, SpawnRequest},
            value::{CommandLine, ProcessName},
        },
    };

    const PANE: u64 = 0;
    /// The configured command of the unsupervised-row tests.
    const SLEEP_COMMAND: &str = "serve";
    /// The command a config change replaces [`SLEEP_COMMAND`] with.
    const OTHER_COMMAND: &str = "serve --again";
    /// A start token no running process has.
    const FOREIGN_TOKEN: u64 = 1;
    /// How long before the test a reconnected process claims to have started.
    const EARLIER: Duration = Duration::from_secs(3600);
    /// Longest a test waits for a scheduled respawn to arrive.
    const RESPAWN_WAIT: Duration = Duration::from_secs(5);

    /// An adopter whose every holder answers with a pid-reporting handle.
    struct FakeAdopter;

    impl HolderAdopter for FakeAdopter {
        fn locate(&self, _child_pid: u32) -> Option<HolderRecord> {
            None
        }

        fn adopt(
            &self,
            _record: &HolderRecord,
            _sink: Box<dyn OutputSink>,
        ) -> Result<Box<dyn ProcessHandle>, PtyError> {
            Ok(Box::new(PidHandle))
        }
    }

    /// A harness over one terminal named `p` that can adopt holders.
    fn adopting(runner: Box<dyn ProcessRunner>) -> Harness {
        let process = Process::builder()
            .id(PaneId::new(PANE))
            .name(ProcessName::try_new("p").unwrap())
            .kind(ProcessKind::Terminal)
            .command(Some(CommandLine::try_new("true").unwrap()))
            .restart(RestartPolicy::Always)
            .build();
        let Harness {
            supervisor,
            events,
            notices,
        } = Harness::new(runner, vec![process]);
        Harness {
            supervisor: supervisor.with_adopter(Box::new(FakeAdopter)),
            events,
            notices,
        }
    }

    /// The record a holder running `process` of kind `kind` in `project` would
    /// leave.
    fn record_in(project: &str, process: &str, kind: ProcessKind) -> HolderRecord {
        record_with_pid(project, process, kind, FAKE_LAUNCHER_PID)
    }

    /// As [`record_in`], for a child running as `child_pid`.
    fn record_with_pid(
        project: &str,
        process: &str,
        kind: ProcessKind,
        child_pid: u32,
    ) -> HolderRecord {
        HolderRecord::builder()
            .id(HolderId::generate().unwrap())
            .project(PathBuf::from(project))
            .process(ProcessName::try_new(process).unwrap())
            .kind(kind)
            .agent_session_id(None)
            .socket(PathBuf::from("/tmp/holder.sock"))
            .holder_pid(1)
            .child_pid(child_pid)
            .child_start_token(None)
            .log(PathBuf::from("/tmp/holder.log"))
            .child_started_at(SystemTime::now())
            .build()
    }

    /// The record of this project's command `p` running as `child_pid`.
    fn command_record(child_pid: u32) -> HolderRecord {
        record_with_pid(TEST_PROJECT, "p", ProcessKind::Command, child_pid)
    }

    /// The record a holder running the terminal `process` of this project
    /// would leave.
    fn record(process: &str) -> HolderRecord {
        record_in(TEST_PROJECT, process, ProcessKind::Terminal)
    }

    #[test]
    fn adopted_process_is_running_with_its_pid_and_restart_policy_applies_on_its_exit() {
        let spawns = SpawnRecorder::default();
        let mut harness = adopting(Box::new(RequestRecordingRunner {
            recorder: spawns.clone(),
        }));
        let pane = PaneId::new(PANE);

        harness.supervisor.adopt(record("p"));
        harness.supervisor.up();

        let status = harness.supervisor.snapshot();
        assert_eq!(*status.processes()[0].state(), ProcessState::Running);
        assert_eq!(*status.processes()[0].pid(), Some(FAKE_LAUNCHER_PID));
        assert!(
            spawns.requests.borrow().is_empty(),
            "up leaves an adopted process running rather than starting another"
        );

        let generation = harness.supervisor.generations()[&pane];
        harness.supervisor.handle_output(
            pane,
            generation,
            ProcessOutput::Exited(ExitOutcome::FAILED_UNKNOWN),
        );

        assert_eq!(
            *harness
                .supervisor
                .workspace()
                .process(pane)
                .unwrap()
                .state(),
            ProcessState::Restarting
        );
        let Ok(ServiceEvent::Respawn {
            pane: respawned,
            generation,
            ..
        }) = harness.events.recv_timeout(RESPAWN_WAIT)
        else {
            panic!("the restart policy schedules a respawn");
        };
        harness.supervisor.handle_respawn(respawned, generation);
        assert_eq!(spawns.requests.borrow().len(), 1);
    }

    /// A holder recorded for another project, or for a process of another kind
    /// under the same name, is refused and left running.
    #[test]
    fn a_record_for_another_project_or_kind_is_refused() {
        let mut harness = adopting(Box::new(FakeRunner));

        let elsewhere = record_in("/elsewhere/muster.yml", "p", ProcessKind::Terminal);
        let other_kind = record_in(TEST_PROJECT, "p", ProcessKind::Command);
        harness.supervisor.adopt(elsewhere.clone());
        harness.supervisor.adopt(other_kind.clone());

        assert!(harness.supervisor.panes().is_empty());
        assert_eq!(harness.notice_messages(), vec![
            with_pids(ADOPT_MISMATCH_NOTICE, &elsewhere),
            with_pids(ADOPT_MISMATCH_NOTICE, &other_kind),
        ]);
        assert!(harness.notice_messages()[0].contains(&FAKE_LAUNCHER_PID.to_string()));
    }

    /// A holder for a process the config no longer has is left running and
    /// reported, never killed.
    #[test]
    fn adopting_an_unknown_process_leaves_it_running_and_says_so() {
        let mut harness = adopting(Box::new(FakeRunner));

        let gone = record("gone");
        harness.supervisor.adopt(gone.clone());

        assert!(harness.supervisor.panes().is_empty());
        assert_eq!(harness.notice_messages(), vec![with_pids(
            ADOPT_UNKNOWN_NOTICE,
            &gone
        )]);
    }

    /// An adopter that finds `record` for its child's pid, and adopts it unless
    /// it `refuses`, counting each adoption.
    struct ReconnectingAdopter {
        record: Option<HolderRecord>,
        refuses: bool,
        adoptions: Rc<Cell<usize>>,
    }

    impl HolderAdopter for ReconnectingAdopter {
        fn locate(&self, child_pid: u32) -> Option<HolderRecord> {
            self.record
                .clone()
                .filter(|record| record.child_pid() == child_pid)
        }

        fn adopt(
            &self,
            record: &HolderRecord,
            _sink: Box<dyn OutputSink>,
        ) -> Result<Box<dyn ProcessHandle>, PtyError> {
            if self.refuses {
                return Err(PtyError::System(REFUSAL.to_string()));
            }
            self.adoptions.set(self.adoptions.get() + 1);
            Ok(Box::new(FixedPidHandle(record.child_pid())))
        }
    }

    /// Why a refusing adopter fails.
    const REFUSAL: &str = "holder unreachable";
    /// Grace the graceful stop policy gives before the kill.
    const STOP_GRACE: Duration = Duration::from_millis(300);
    /// Longest a test waits for an unsupervised stop to settle.
    const SETTLE_WAIT: Duration = Duration::from_secs(10);

    /// A handle that reports a fixed pid and accepts every request.
    struct FixedPidHandle(u32);

    impl ProcessHandle for FixedPidHandle {
        fn process_id(&self) -> Option<u32> {
            Some(self.0)
        }

        fn write_input(&mut self, _bytes: &[u8]) -> Result<(), PtyError> {
            Ok(())
        }

        fn resize(&mut self, _size: PtySize) -> Result<(), PtyError> {
            Ok(())
        }

        fn pause(&mut self) -> Result<(), PtyError> {
            Ok(())
        }

        fn resume(&mut self) -> Result<(), PtyError> {
            Ok(())
        }

        fn kill(&mut self) -> Result<(), PtyError> {
            Ok(())
        }
    }

    /// A runner whose every process reports `pid`, counting its spawns.
    struct FixedPidRunner {
        pid: u32,
        spawns: Arc<AtomicUsize>,
    }

    impl ProcessRunner for FixedPidRunner {
        fn spawn(
            &self,
            _request: SpawnRequest,
            _sink: Box<dyn OutputSink>,
        ) -> Result<Box<dyn ProcessHandle>, PtyError> {
            self.spawns.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(FixedPidHandle(self.pid)))
        }
    }

    /// The graceful stop policy of the unsupervised tests.
    fn graceful() -> StopPolicy {
        StopPolicy::builder()
            .signal(StopSignal::Interrupt)
            .grace_period(STOP_GRACE)
            .build()
    }

    /// A harness over `p` of `kind` with `stop` policy and restart `Always`,
    /// whose processes report `pid` and whose holder `adopter` can reconnect
    /// to, plus its spawn count. Nothing is started.
    fn reconnectable_over(
        kind: ProcessKind,
        stop: Option<StopPolicy>,
        pid: u32,
        adopter: ReconnectingAdopter,
    ) -> (Harness, Arc<AtomicUsize>) {
        let process = Process::builder()
            .id(PaneId::new(PANE))
            .name(ProcessName::try_new("p").unwrap())
            .kind(kind)
            .command(Some(CommandLine::try_new("true").unwrap()))
            .restart(RestartPolicy::Always)
            .stop(stop)
            .build();
        let spawns = Arc::new(AtomicUsize::new(0));
        let Harness {
            supervisor,
            events,
            notices,
        } = Harness::new(
            Box::new(FixedPidRunner {
                pid,
                spawns: spawns.clone(),
            }),
            vec![process],
        );
        let harness = Harness {
            supervisor: supervisor.with_adopter(Box::new(adopter)),
            events,
            notices,
        };
        (harness, spawns)
    }

    /// As [`reconnectable_over`], started, with a pid no test relies on.
    fn reconnectable(
        kind: ProcessKind,
        stop: Option<StopPolicy>,
        adopter: ReconnectingAdopter,
    ) -> Harness {
        let (mut harness, _) = reconnectable_over(kind, stop, FAKE_LAUNCHER_PID, adopter);
        harness.supervisor.up();
        harness
    }

    /// Drives the force-stop timers of an unsupervised stop until `p` settles,
    /// failing the test after [`SETTLE_WAIT`].
    fn drive_until_exited(harness: &mut Harness) {
        drive_until(harness, "the stop to settle", |harness| {
            state(harness) == ProcessState::Exited
        });
    }

    /// Hands every force-stop deadline that arrives to the supervisor until
    /// `done` holds, failing the test after [`SETTLE_WAIT`].
    fn drive_until(harness: &mut Harness, what: &str, done: impl Fn(&Harness) -> bool) {
        let deadline = Instant::now() + SETTLE_WAIT;
        while !done(harness) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match harness.events.recv_timeout(remaining) {
                Ok(ServiceEvent::ForceStop {
                    pane,
                    spawn_generation,
                    shutdown_generation,
                    ..
                }) => harness.supervisor.handle_force_stop(
                    pane,
                    spawn_generation,
                    shutdown_generation,
                ),
                Ok(_) => {},
                Err(_) => panic!("timed out waiting for {what}"),
            }
        }
    }

    /// A supervisor of the configured command `p` (graceful stop) whose
    /// registry reads that config, with `p` left running unsupervised as
    /// `orphan` by an adoption that failed at startup, plus its spawn count.
    fn configured_unsupervised(orphan: &Orphan) -> (Harness, Arc<AtomicUsize>, WorkspaceConfig) {
        let config = command_config(SLEEP_COMMAND);
        let spawns = Arc::new(AtomicUsize::new(0));
        let Harness {
            supervisor,
            events,
            notices,
        } = Harness::with_registry(
            Box::new(FixedPidRunner {
                pid: orphan.pid(),
                spawns: spawns.clone(),
            }),
            config.to_processes(),
            Box::new(FakeRegistry {
                projects: Vec::new(),
                workspace: config.clone(),
                recorder: Recorder::default(),
            }),
        );
        let mut harness = Harness {
            supervisor: supervisor.with_adopter(Box::new(ReconnectingAdopter {
                record: None,
                refuses: true,
                adoptions: Rc::new(Cell::new(0)),
            })),
            events,
            notices,
        };
        harness.supervisor.adopt(command_record(orphan.pid()));
        assert_eq!(state(&harness), ProcessState::Unsupervised);
        (harness, spawns, config)
    }

    /// A config of one command `p` running `command` with the graceful stop.
    fn command_config(command: &str) -> WorkspaceConfig {
        WorkspaceConfig::builder()
            .agents(vec![])
            .terminals(vec![])
            .commands(vec![
                ProcessSpec::builder()
                    .name(ProcessName::try_new("p").unwrap())
                    .command(Some(CommandLine::try_new(command).unwrap()))
                    .stop(Some(graceful()))
                    .autostart(false)
                    .build(),
            ])
            .build()
    }

    /// The shared assertions on an unsupervised `p` running as `orphan`: it is
    /// reported with its pid, refuses to start or restart a second copy, and a
    /// stop signals it but reports `Exited` only once the pid is gone.
    fn assert_unsupervised_until_gone(
        harness: &mut Harness,
        spawns: &AtomicUsize,
        orphan: &Orphan,
    ) {
        let name = ProcessName::try_new("p").unwrap();
        assert_eq!(state(harness), ProcessState::Unsupervised);
        assert!(!harness.supervisor.is_live(PaneId::new(PANE)));
        assert_eq!(
            *harness.supervisor.snapshot().processes()[0].pid(),
            Some(orphan.pid())
        );
        let spawned = spawns.load(Ordering::SeqCst);

        harness.supervisor.start(&name);
        harness.supervisor.restart(&name);

        assert_eq!(spawns.load(Ordering::SeqCst), spawned, "no second copy");
        assert_eq!(state(harness), ProcessState::Unsupervised);
        assert!(
            harness
                .notice_messages()
                .iter()
                .any(|message| message.ends_with(&orphan.pid().to_string()))
        );

        harness.supervisor.stop(&name);
        assert_eq!(state(harness), ProcessState::Stopping);
        assert!(orphan.alive(), "the graceful signal is ignored");

        drive_until_exited(harness);
        assert!(!orphan.alive(), "exited only once the pid is gone");
    }

    /// The current generation of [`PANE`].
    fn generation(harness: &Harness) -> SpawnGeneration {
        harness.supervisor.generations()[&PaneId::new(PANE)]
    }

    /// The state of `p`.
    fn state(harness: &Harness) -> ProcessState {
        *harness
            .supervisor
            .workspace()
            .process(PaneId::new(PANE))
            .unwrap()
            .state()
    }

    /// Reports `p`'s output stream as broken.
    fn break_stream(harness: &mut Harness) {
        let current = generation(harness);
        harness.supervisor.handle_output(
            PaneId::new(PANE),
            current,
            ProcessOutput::Unreadable("bad frame".to_string()),
        );
    }

    /// A broken stream reconnects to the holder: output flows again on the new
    /// connection, the old one's is stale, and a later stop observes the exit.
    #[test]
    fn an_unreadable_stream_reconnects_and_a_later_stop_observes_the_exit() {
        let adoptions = Rc::new(Cell::new(0));
        let mut harness = reconnectable(ProcessKind::Terminal, None, ReconnectingAdopter {
            record: Some(record("p")),
            refuses: false,
            adoptions: adoptions.clone(),
        });
        let pane = PaneId::new(PANE);
        let broken = generation(&harness);

        break_stream(&mut harness);

        assert_eq!(adoptions.get(), 1);
        assert!(harness.supervisor.is_live(pane));
        assert_eq!(state(&harness), ProcessState::Running);
        let current = generation(&harness);
        assert_ne!(current, broken);
        harness
            .supervisor
            .handle_output(pane, broken, ProcessOutput::Chunk(b"stale".to_vec()));
        harness
            .supervisor
            .handle_output(pane, current, ProcessOutput::Chunk(b"fresh".to_vec()));
        let screen = harness.supervisor.panes()[&pane].parser().screen_text();
        assert!(screen.contains("fresh") && !screen.contains("stale"));

        harness.supervisor.stop(&ProcessName::try_new("p").unwrap());
        let current = generation(&harness);
        harness.supervisor.handle_output(
            pane,
            current,
            ProcessOutput::Exited(ExitOutcome::FAILED_UNKNOWN),
        );
        assert_eq!(state(&harness), ProcessState::Exited);
        assert!(!harness.supervisor.is_live(pane));
    }

    /// A graceful stop already delivered when the stream breaks carries over
    /// the reconnect, so the exit the holder reports on the new connection
    /// settles as a stop rather than a restart and never strands the process
    /// in `Stopping`.
    #[test]
    fn a_stop_in_flight_survives_a_reconnect() {
        let stop = StopPolicy::builder()
            .signal(StopSignal::Interrupt)
            .grace_period(Duration::from_secs(7))
            .build();
        let mut harness = reconnectable(ProcessKind::Command, Some(stop), ReconnectingAdopter {
            record: Some(record_in(TEST_PROJECT, "p", ProcessKind::Command)),
            refuses: false,
            adoptions: Rc::new(Cell::new(0)),
        });
        harness.supervisor.stop(&ProcessName::try_new("p").unwrap());
        assert_eq!(state(&harness), ProcessState::Stopping);

        break_stream(&mut harness);
        assert_eq!(state(&harness), ProcessState::Stopping);
        let current = generation(&harness);
        harness.supervisor.handle_output(
            PaneId::new(PANE),
            current,
            ProcessOutput::Exited(ExitOutcome::FAILED_UNKNOWN),
        );

        assert_eq!(state(&harness), ProcessState::Exited);
        assert!(
            harness.events.try_recv().is_err(),
            "no restart is scheduled"
        );
    }

    /// A broken stream whose holder cannot be found or adopted again, while
    /// its child still runs, leaves the process unsupervised: no second copy
    /// starts, and a stop signals the child and reports the exit only once the
    /// pid is gone.
    #[test]
    fn a_failed_reconnect_leaves_the_process_unsupervised_until_its_pid_is_gone() {
        for refuses in [false, true] {
            let orphan = Orphan::spawn();
            let record = command_record(orphan.pid());
            let (mut harness, spawns) = reconnectable_over(
                ProcessKind::Command,
                Some(graceful()),
                orphan.pid(),
                ReconnectingAdopter {
                    record: refuses.then_some(record),
                    refuses,
                    adoptions: Rc::new(Cell::new(0)),
                },
            );
            harness.supervisor.up();

            break_stream(&mut harness);

            assert!(
                harness
                    .notice_messages()
                    .iter()
                    .any(|message| message.starts_with(ADOPT_FAILED_NOTICE))
            );
            assert_unsupervised_until_gone(&mut harness, &spawns, &orphan);
        }
    }

    /// A holder that cannot be adopted at startup, while its child runs, is
    /// unsupervised exactly as a failed reconnect is, and `up` starts no second
    /// copy beside it.
    #[test]
    fn a_holder_unadoptable_at_startup_is_unsupervised_until_its_pid_is_gone() {
        let orphan = Orphan::spawn();
        let (mut harness, spawns) = reconnectable_over(
            ProcessKind::Command,
            Some(graceful()),
            orphan.pid(),
            ReconnectingAdopter {
                record: None,
                refuses: true,
                adoptions: Rc::new(Cell::new(0)),
            },
        );
        harness.supervisor.adopt(command_record(orphan.pid()));
        harness.supervisor.up();

        assert_eq!(spawns.load(Ordering::SeqCst), 0, "up starts no second copy");
        assert_unsupervised_until_gone(&mut harness, &spawns, &orphan);
    }

    /// A failed reconnect whose child is verifiably gone is said so and
    /// settles as a failed exit: the restart policy applies, here a respawn.
    #[test]
    fn a_failed_reconnect_of_a_gone_child_settles_as_an_exit() {
        let orphan = Orphan::spawn();
        let pid = orphan.pid();
        drop(orphan);
        let (mut harness, spawns) =
            reconnectable_over(ProcessKind::Terminal, None, pid, ReconnectingAdopter {
                record: None,
                refuses: false,
                adoptions: Rc::new(Cell::new(0)),
            });
        harness.supervisor.up();

        let effect = harness.supervisor.handle_output(
            PaneId::new(PANE),
            generation(&harness),
            ProcessOutput::Unreadable("bad frame".to_string()),
        );

        assert!(matches!(effect, OutputEffect::Exited {
            retired: false,
            ..
        }));
        assert_eq!(state(&harness), ProcessState::Restarting);
        assert!(harness.notice_messages().iter().any(|message| {
            message.starts_with(ADOPT_FAILED_NOTICE)
                && message.contains(SINCE_EXITED)
                && message.ends_with(&pid.to_string())
        }));
        let Ok(ServiceEvent::Respawn {
            pane, generation, ..
        }) = harness.events.recv_timeout(RESPAWN_WAIT)
        else {
            panic!("the restart policy schedules a respawn");
        };
        harness.supervisor.handle_respawn(pane, generation);
        assert_eq!(spawns.load(Ordering::SeqCst), 2);
    }

    /// A reconnect keeps what outlives a connection: a process removed from
    /// muster.yml still retires on its exit instead of respawning, and its
    /// uptime still counts from its real start.
    #[test]
    fn a_reconnect_keeps_config_membership_and_start_time() {
        let mut harness = reconnectable(ProcessKind::Terminal, None, ReconnectingAdopter {
            record: Some(record("p")),
            refuses: false,
            adoptions: Rc::new(Cell::new(0)),
        });
        let pane = PaneId::new(PANE);
        let started_at = Instant::now() - EARLIER;
        if let Some(target) = harness.supervisor.panes.get_mut(&pane) {
            target.config_membership = ConfigMembership::RetireOnExit;
            target.started_at = started_at;
        }

        break_stream(&mut harness);

        let target = &harness.supervisor.panes()[&pane];
        assert_eq!(target.config_membership(), ConfigMembership::RetireOnExit);
        assert!(
            harness.supervisor.snapshot().processes()[0]
                .uptime()
                .unwrap()
                >= EARLIER
        );
        let current = generation(&harness);
        harness.supervisor.handle_output(
            pane,
            current,
            ProcessOutput::Exited(ExitOutcome::FAILED_UNKNOWN),
        );
        assert!(harness.supervisor.workspace().process(pane).is_none());
    }

    /// A paused process reconnects paused, so a resume still reaches it.
    #[test]
    fn a_paused_process_reconnects_paused_and_resumes() {
        let mut harness = reconnectable(ProcessKind::Terminal, None, ReconnectingAdopter {
            record: Some(record("p")),
            refuses: false,
            adoptions: Rc::new(Cell::new(0)),
        });
        let name = ProcessName::try_new("p").unwrap();
        harness.supervisor.pause(&name);
        assert_eq!(state(&harness), ProcessState::Paused);

        break_stream(&mut harness);
        assert_eq!(state(&harness), ProcessState::Paused);

        harness.supervisor.resume(&name);
        assert_eq!(state(&harness), ProcessState::Running);
    }

    /// A changed command replaces the spec, but the old row keeps the
    /// unsupervised pid until the child is gone, and no launch of the same
    /// process starts a second copy beside it meanwhile.
    #[test]
    fn a_changed_command_keeps_the_unsupervised_pid_until_the_child_is_gone() {
        let orphan = Orphan::spawn();
        let (mut harness, spawns, _) = configured_unsupervised(&orphan);
        let old = PaneId::new(PANE);

        harness
            .supervisor
            .reconcile_config(&command_config(OTHER_COMMAND));

        assert_eq!(
            *harness.supervisor.snapshot().processes()[0].pid(),
            Some(orphan.pid())
        );
        assert_eq!(
            harness.supervisor.panes()[&old].config_membership(),
            ConfigMembership::RetireOnExit
        );
        let replacement = harness
            .supervisor
            .workspace()
            .processes()
            .iter()
            .map(|process| *process.id())
            .find(|pane| *pane != old)
            .expect("the new spec has its own row");
        harness.supervisor.start_pane(replacement);
        harness.supervisor.up();
        assert_eq!(spawns.load(Ordering::SeqCst), 0, "no second copy");

        harness.supervisor.stop_pane(old);
        drive_until(&mut harness, "the old row to retire", |harness| {
            harness.supervisor.workspace().process(old).is_none()
        });
        harness.supervisor.start_pane(replacement);
        assert_eq!(spawns.load(Ordering::SeqCst), 1, "the replacement starts");
    }

    /// An unsupervised child that exits on its own no longer blocks a start.
    #[test]
    fn an_orphan_that_exits_on_its_own_no_longer_blocks_a_start() {
        let orphan = Orphan::spawn();
        let (mut harness, spawns, _) = configured_unsupervised(&orphan);
        assert_eq!(orphan.end_with(libc::SIGKILL), Some(libc::SIGKILL));

        assert_eq!(*harness.supervisor.snapshot().processes()[0].pid(), None);
        harness
            .supervisor
            .start(&ProcessName::try_new("p").unwrap());

        assert_eq!(spawns.load(Ordering::SeqCst), 1);
        assert_eq!(state(&harness), ProcessState::Running);
    }

    /// A pid now held by a process with another start token is not the
    /// unsupervised child: a stop settles it as gone and never signals it.
    #[test]
    fn a_reused_pid_is_never_signalled() {
        let orphan = Orphan::spawn();
        let (mut harness, ..) = configured_unsupervised(&orphan);
        if let Some(target) = harness.supervisor.panes.get_mut(&PaneId::new(PANE)) {
            target.unsupervised = Some(
                UnsupervisedChild::builder()
                    .pid(orphan.pid())
                    .start_token(Some(
                        AgentProcessStartToken::try_new(FOREIGN_TOKEN).unwrap(),
                    ))
                    .build(),
            );
        }

        harness
            .supervisor
            .force_kill(&ProcessName::try_new("p").unwrap());

        assert_eq!(state(&harness), ProcessState::Exited);
        assert_eq!(
            orphan.end_with(libc::SIGUSR1),
            Some(libc::SIGUSR1),
            "the stop sent the reused pid nothing"
        );
    }

    /// A stop in flight when the stream breaks gets a fresh force-stop
    /// deadline on the new connection, so a child ignoring the stop signal is
    /// still killed rather than left `Stopping`.
    #[test]
    fn a_reconnect_rearms_the_deadline_of_a_stop_in_flight() {
        let mut harness = reconnectable(
            ProcessKind::Command,
            Some(graceful()),
            ReconnectingAdopter {
                record: Some(record_in(TEST_PROJECT, "p", ProcessKind::Command)),
                refuses: false,
                adoptions: Rc::new(Cell::new(0)),
            },
        );
        harness.supervisor.stop(&ProcessName::try_new("p").unwrap());

        break_stream(&mut harness);

        drive_until(&mut harness, "the re-armed force stop", |harness| {
            harness
                .notice_messages()
                .iter()
                .any(|message| message == FORCE_STOP_NOTICE)
        });
    }

    /// A child that survives every kill, here a zombie nobody reaps, ends the
    /// stop after the kill limit: an error names it and the process returns to
    /// `Unsupervised` with the stop retryable.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_child_that_survives_every_kill_leaves_the_stop_retryable() {
        let orphan = Orphan::unreaped();
        let (mut harness, ..) = configured_unsupervised(&orphan);
        let name = ProcessName::try_new("p").unwrap();

        harness.supervisor.stop(&name);
        drive_until(&mut harness, "the kills to give up", |harness| {
            state(harness) == ProcessState::Unsupervised
        });

        assert!(
            harness
                .notice_messages()
                .iter()
                .any(|message| message.ends_with(&orphan.pid().to_string()))
        );
        harness.supervisor.stop(&name);
        assert_eq!(
            state(&harness),
            ProcessState::Stopping,
            "the stop is retried"
        );
    }

    /// `ps` never shows an unsupervised process with no pid: a child found
    /// gone shows as exited, and one whose row left muster.yml shows no row.
    #[test]
    fn a_gone_unsupervised_child_shows_as_exited_or_as_no_row() {
        let orphan = Orphan::spawn();
        let (harness, ..) = configured_unsupervised(&orphan);
        assert_eq!(orphan.end_with(libc::SIGKILL), Some(libc::SIGKILL));

        let status = harness.supervisor.snapshot();
        assert_eq!(*status.processes()[0].state(), ProcessState::Exited);
        assert_eq!(*status.processes()[0].pid(), None);

        let removed = Orphan::spawn();
        let (mut harness, ..) = configured_unsupervised(&removed);
        harness
            .supervisor
            .reconcile_config(&command_config(OTHER_COMMAND));
        assert_eq!(removed.end_with(libc::SIGKILL), Some(libc::SIGKILL));

        let status = harness.supervisor.snapshot();
        assert_eq!(status.processes().len(), 1, "only the replacement row");
        assert_eq!(*status.processes()[0].state(), ProcessState::Pending);
    }

    /// Starting a row already removed from muster.yml whose unsupervised
    /// child has since exited says the row was removed, and starts nothing.
    #[test]
    fn starting_a_removed_row_whose_child_is_gone_says_it_was_removed() {
        let orphan = Orphan::spawn();
        let (mut harness, spawns, _) = configured_unsupervised(&orphan);
        let old = PaneId::new(PANE);
        harness
            .supervisor
            .reconcile_config(&command_config(OTHER_COMMAND));
        assert_eq!(orphan.end_with(libc::SIGKILL), Some(libc::SIGKILL));

        harness.supervisor.start_pane(old);

        assert!(harness.supervisor.workspace().process(old).is_none());
        assert_eq!(spawns.load(Ordering::SeqCst), 0);
        assert_eq!(
            harness.notice_messages().last().map(String::as_str),
            Some(UNSUPERVISED_ROW_REMOVED)
        );
    }

    /// A holder record whose child pid now belongs to another process is not
    /// mistaken for its child when adoption fails: nothing is left
    /// unsupervised, the notice says the child exited, and a start goes ahead.
    #[test]
    fn an_unadoptable_record_whose_child_pid_was_reused_leaves_nothing_unsupervised() {
        let stranger = Orphan::spawn();
        let (mut harness, spawns) = reconnectable_over(
            ProcessKind::Command,
            Some(graceful()),
            stranger.pid(),
            ReconnectingAdopter {
                record: None,
                refuses: true,
                adoptions: Rc::new(Cell::new(0)),
            },
        );
        let record = HolderRecord::builder()
            .id(HolderId::generate().unwrap())
            .project(PathBuf::from(TEST_PROJECT))
            .process(ProcessName::try_new("p").unwrap())
            .kind(ProcessKind::Command)
            .agent_session_id(None)
            .socket(PathBuf::from("/tmp/holder.sock"))
            .holder_pid(1)
            .child_pid(stranger.pid())
            .child_start_token(Some(
                AgentProcessStartToken::try_new(FOREIGN_TOKEN).unwrap(),
            ))
            .log(PathBuf::from("/tmp/holder.log"))
            .child_started_at(SystemTime::now())
            .build();

        harness.supervisor.adopt(record);

        assert_ne!(state(&harness), ProcessState::Unsupervised);
        assert!(
            harness
                .notice_messages()
                .iter()
                .any(|message| message.contains(SINCE_EXITED))
        );
        harness
            .supervisor
            .start(&ProcessName::try_new("p").unwrap());
        assert_eq!(spawns.load(Ordering::SeqCst), 1);
        assert!(stranger.alive(), "the stranger is never signalled");
    }

    /// An adopter that runs on a worker, as the service's does, finds
    /// `record` for its child's pid and fails every adoption.
    #[derive(Clone)]
    struct FailingWorkerAdopter {
        record: Option<HolderRecord>,
    }

    impl HolderAdopter for FailingWorkerAdopter {
        fn locate(&self, child_pid: u32) -> Option<HolderRecord> {
            self.record
                .clone()
                .filter(|record| record.child_pid() == child_pid)
        }

        fn adopt(
            &self,
            _record: &HolderRecord,
            _sink: Box<dyn OutputSink>,
        ) -> Result<Box<dyn ProcessHandle>, PtyError> {
            Err(PtyError::System(REFUSAL.to_string()))
        }

        fn detached(&self) -> Option<Arc<dyn HolderAdopter + Send + Sync>> {
            Some(Arc::new(self.clone()))
        }
    }

    /// The adoption result a worker posted for `harness`, skipping any other
    /// event.
    fn adopted(
        harness: &Harness,
    ) -> (
        PaneId,
        SpawnGeneration,
        HolderRecord,
        Result<Box<dyn ProcessHandle>, PtyError>,
    ) {
        let deadline = Instant::now() + SETTLE_WAIT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match harness.events.recv_timeout(remaining) {
                Ok(ServiceEvent::Adopted {
                    pane,
                    generation,
                    record,
                    result,
                    ..
                }) => return (pane, generation, *record, result),
                Ok(_) => {},
                Err(_) => panic!("the worker posts the adoption result"),
            }
        }
    }

    /// Closing an agent session while its holder is being adopted keeps the
    /// kill and the retirement for the adoption: when it fails with the child
    /// still running, the child is killed and the row retires once it is
    /// gone, so no child is left running without a row.
    #[test]
    fn closing_a_session_while_its_adoption_fails_kills_the_child_and_retires_the_row() {
        let (mut harness, _sessions, _spawns) = agent_harness(Vec::new());
        let pane = harness
            .supervisor
            .create_agent_session(AgentTool::Codex, None, None, None)
            .unwrap()
            .pane();
        let generation = harness.supervisor.generations()[&pane];
        harness.supervisor.handle_output(
            pane,
            generation,
            ProcessOutput::Exited(ExitOutcome::Succeeded),
        );
        let name = harness
            .supervisor
            .workspace()
            .process(pane)
            .expect("an exited session keeps its row")
            .name()
            .clone();
        let orphan = Orphan::spawn();
        let record = record_with_pid(
            TEST_PROJECT,
            name.as_ref(),
            ProcessKind::Agent,
            orphan.pid(),
        );
        harness.supervisor.adopter = Some(Box::new(FailingWorkerAdopter { record: None }));
        harness.supervisor.adopt(record);
        assert_eq!(state_of(&harness, pane), Some(ProcessState::Starting));

        harness.supervisor.close_agent_session(&name);
        assert!(
            harness.supervisor.workspace().process(pane).is_some(),
            "the row stays while its adoption is in flight"
        );
        let (pane, generation, record, result) = adopted(&harness);
        harness
            .supervisor
            .handle_adopted(pane, generation, &record, result);

        drive_until(&mut harness, "the closed row to retire", |harness| {
            harness.supervisor.workspace().process(pane).is_none()
        });
        assert!(!orphan.alive(), "the child was killed");
    }

    /// A restart asked for while a reconnect was in flight is applied when
    /// the reconnect fails on a child that has exited and settles that exit.
    #[test]
    fn a_restart_kept_through_a_failed_reconnect_is_applied() {
        let gone = Orphan::spawn();
        let pid = gone.pid();
        drop(gone);
        let (mut harness, spawns) =
            reconnectable_over(ProcessKind::Terminal, None, pid, ReconnectingAdopter {
                record: None,
                refuses: false,
                adoptions: Rc::new(Cell::new(0)),
            });
        harness.supervisor.adopter = Some(Box::new(FailingWorkerAdopter {
            record: Some(record_with_pid(
                TEST_PROJECT,
                "p",
                ProcessKind::Terminal,
                pid,
            )),
        }));
        harness.supervisor.up();
        assert_eq!(spawns.load(Ordering::SeqCst), 1);

        break_stream(&mut harness);
        assert_eq!(state(&harness), ProcessState::Starting);
        harness
            .supervisor
            .restart(&ProcessName::try_new("p").unwrap());
        let (pane, generation, record, result) = adopted(&harness);
        harness
            .supervisor
            .handle_adopted(pane, generation, &record, result);

        assert_eq!(spawns.load(Ordering::SeqCst), 2, "the kept restart ran");
        assert_eq!(state_of(&harness, pane), Some(ProcessState::Running));
    }

    /// A pause asked for while a reconnect was in flight belongs to the
    /// process that reconnect was for: when it fails and the restart already
    /// in flight launches a fresh process, that process is not paused.
    #[test]
    fn a_pause_kept_through_a_failed_reconnect_never_reaches_the_relaunched_process() {
        let gone = Orphan::spawn();
        let pid = gone.pid();
        drop(gone);
        let (mut harness, spawns) =
            reconnectable_over(ProcessKind::Terminal, None, pid, ReconnectingAdopter {
                record: None,
                refuses: false,
                adoptions: Rc::new(Cell::new(0)),
            });
        harness.supervisor.adopter = Some(Box::new(FailingWorkerAdopter {
            record: Some(record_with_pid(
                TEST_PROJECT,
                "p",
                ProcessKind::Terminal,
                pid,
            )),
        }));
        harness.supervisor.up();
        let process = ProcessName::try_new("p").unwrap();
        harness.supervisor.restart(&process);

        break_stream(&mut harness);
        assert_eq!(state(&harness), ProcessState::Starting);
        harness.supervisor.pause(&process);
        let (pane, generation, record, result) = adopted(&harness);
        harness
            .supervisor
            .handle_adopted(pane, generation, &record, result);

        assert_eq!(
            spawns.load(Ordering::SeqCst),
            2,
            "the restart in flight ran"
        );
        assert_eq!(state_of(&harness, pane), Some(ProcessState::Running));
    }

    /// The state of the process on `pane`, while it has a row.
    fn state_of(harness: &Harness, pane: PaneId) -> Option<ProcessState> {
        harness
            .supervisor
            .workspace()
            .process(pane)
            .map(|process| *process.state())
    }

    /// A stop in flight when the stream breaks, whose holder cannot be reached
    /// again while the child still runs, carries on against the child
    /// directly instead of being dropped: the process stays `Stopping` and
    /// settles once the child is gone.
    #[test]
    fn a_stop_in_flight_continues_when_the_reconnect_fails_while_the_child_runs() {
        let orphan = Orphan::spawn();
        let (mut harness, _) = reconnectable_over(
            ProcessKind::Command,
            Some(graceful()),
            orphan.pid(),
            ReconnectingAdopter {
                record: None,
                refuses: false,
                adoptions: Rc::new(Cell::new(0)),
            },
        );
        harness.supervisor.up();
        harness.supervisor.stop(&ProcessName::try_new("p").unwrap());
        assert_eq!(state(&harness), ProcessState::Stopping);

        break_stream(&mut harness);

        assert_eq!(state(&harness), ProcessState::Stopping);
        drive_until_exited(&mut harness);
        assert!(!orphan.alive());
    }
}
