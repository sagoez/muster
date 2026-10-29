use std::{sync::Arc, thread};

use super::{
    adoption::Adoption, carried_pane::CarriedPane, channel_output_sink::ChannelOutputSink,
    config_membership::ConfigMembership, entity::Supervisor, launch_purpose::LaunchPurpose,
    pending_intent::PendingIntent, pending_launch::PendingLaunch,
    spawn_generation::SpawnGeneration, unsupervised_child::UnsupervisedChild,
};
use crate::{
    adapter::{
        holder::{HolderRecord, HolderRunner, abandon},
        service::{NoticeLevel, ServiceEvent},
    },
    domain::{
        config::{Environment, ProcessSpec},
        port::{OutputSink, ProcessHandle},
        process::{ExitIntent, ProcessState, RestartPolicy},
        pty::{ExitOutcome, PtyError},
        value::{OutputCursor, PaneId, Pid, ProcessName, ProjectRoot},
    },
};

/// Shown when a running holder's process is no longer in the project, so it
/// is adopted only to be stopped through the stop policy its record carries.
pub(super) const ADOPT_REMOVED_NOTICE: &str = "stopped: removed from the project";
/// Shown when a holder record names a process that is already supervised and
/// whose child still runs.
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
/// the process.
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

/// Tells the holder `record` names, whose child is gone, to hand over the
/// exit it holds and finish, off the loop. A holder no service adopts would
/// otherwise wait for its exit to be collected forever.
fn collect_exit(record: &HolderRecord) {
    let socket = record.socket().clone();
    thread::spawn(move || abandon(&socket));
}

/// Whether the child `record` names still runs as that same process; when it
/// does not, its holder's exit is collected.
fn child_runs_or_collect(record: &HolderRecord) -> bool {
    let running =
        UnsupervisedChild::observe_expected(record.child_pid(), record.child_start_token())
            .is_some();
    if !running {
        collect_exit(record);
    }
    running
}

/// Takes over a holder a previous service left running, so its process is
/// supervised again without being restarted, and finds the holder of a running
/// process.
pub trait HolderAdopter {
    /// The record of the holder running the child with `child_pid`, if a
    /// holder runs it.
    fn locate(&self, child_pid: Pid) -> Option<HolderRecord>;

    /// The record of a holder that runs the child with `child_pid` and serves
    /// a supervisor now, if one does.
    fn serving(&self, child_pid: Pid) -> Option<HolderRecord>;

    /// Becomes the supervisor of the holder `record` describes, forwarding its
    /// screen, output and exit to `sink`, with the output it logged after
    /// `since`, or since its run began, as the handle's backlog.
    ///
    /// # Errors
    /// Returns a [`PtyError`] when the holder cannot be reached or sends no
    /// screen.
    fn adopt(
        &self,
        record: &HolderRecord,
        since: Option<OutputCursor>,
        sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError>;

    /// Where the log of the process `name` of `project` ends, and its last
    /// bytes before that with where they start, for a process this service
    /// never saw run. Defaults to none: no log is known.
    fn log_tail(
        &self,
        _project: &ProjectRoot,
        _name: &ProcessName,
    ) -> Option<(OutputCursor, Vec<u8>)> {
        None
    }

    /// This adopter, shareable with a worker thread, when an adoption may
    /// block long enough that the supervisor must not wait for it on its
    /// loop. Defaults to none: adoptions run inline.
    fn detached(&self) -> Option<Arc<dyn HolderAdopter + Send + Sync>> {
        None
    }
}

impl HolderAdopter for HolderRunner {
    fn locate(&self, child_pid: Pid) -> Option<HolderRecord> {
        HolderRunner::locate(self, child_pid)
    }

    fn serving(&self, child_pid: Pid) -> Option<HolderRecord> {
        HolderRunner::serving(self, child_pid)
    }

    fn adopt(
        &self,
        record: &HolderRecord,
        since: Option<OutputCursor>,
        sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        HolderRunner::adopt(self, record, since, sink)
    }

    fn log_tail(
        &self,
        project: &ProjectRoot,
        name: &ProcessName,
    ) -> Option<(OutputCursor, Vec<u8>)> {
        HolderRunner::log_tail(self, project, name)
    }
}

impl Supervisor {
    /// Supervises the running holder `record` describes as its named process,
    /// without restarting it, and with the environment it was launched with
    /// for its policy restarts. A process no longer in the project is adopted
    /// only to be stopped through the stop policy its record carries, with a
    /// notice, and its row goes once it exits. A second copy of a supervised
    /// process is left running and said so; one whose child is gone has its
    /// exit collected so its holder finishes.
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
        self.settle_adoption(pane, generation, record, result);
    }

    /// Reconnects to the holder of `pane`'s process after its output stream
    /// broke: drops the handle, then adopts the holder again, which resyncs the
    /// screen from a fresh snapshot. Everything about the pane that outlives a
    /// connection carries over: its state, exit intent, stack membership and
    /// start time, and a stop in flight gets a fresh force-stop deadline. When
    /// the holder cannot be adopted and the child still runs, the process is
    /// left unsupervised, as an unadoptable holder is at startup; a child that
    /// is gone settles as a failed exit under the pane's policy. A failed
    /// reconnect always raises a notice. When the adopter allows it, finding
    /// the holder and adopting it both run off the loop: the pane is
    /// `Starting` meanwhile, keeps what is asked of it, and settles in
    /// [`Self::handle_reconnect_located`] and then [`Self::handle_adopted`].
    pub(super) fn reconnect(&mut self, pane: PaneId) {
        let state = self.workspace.process(pane).map(|process| *process.state());
        let Some(carried) = self
            .panes
            .get_mut(&pane)
            .and_then(|target| target.carry_over(state))
        else {
            return;
        };
        let detached = self.adopter.as_ref().and_then(|adopter| adopter.detached());
        if let (Some(pid), Some(detached)) = (carried.pid(), detached) {
            let generation = self.bump_generation(pane);
            let prior_state = state.unwrap_or(ProcessState::Pending);
            self.launches.begin(
                pane,
                PendingLaunch::new(generation, prior_state, LaunchPurpose::Adopt {
                    carried: Some(carried),
                }),
            );
            self.workspace.set_state(pane, ProcessState::Starting);
            let events = self.events.clone();
            let project = self.root().clone();
            thread::spawn(move || {
                let record = detached.locate(pid).map(Box::new);
                let _ = events.send(ServiceEvent::ReconnectLocated {
                    project,
                    pane,
                    generation,
                    record,
                });
            });
            return;
        }
        let record = carried
            .pid()
            .and_then(|pid| self.adopter.as_ref()?.locate(pid));
        match record {
            Some(record) => self.begin_adoption(&record, Some((pane, carried))),
            None => {
                self.bump_generation(pane);
                self.reconnect_failed(pane, carried, None, None);
            },
        }
    }

    /// Settles a worker's look for the holder of `pane`'s process, whose
    /// stream broke, for the reconnect at `generation`: adopts `record` again
    /// on a worker, keeping what was asked meanwhile, or settles the
    /// reconnect as failed when no holder runs that child, then applies what
    /// was asked. A reconnect a later action or a shutdown superseded adopts
    /// nothing; a child it found still running is said so.
    pub fn handle_reconnect_located(
        &mut self,
        pane: PaneId,
        generation: SpawnGeneration,
        record: Option<HolderRecord>,
    ) {
        let detaching = self.launches.is_detaching();
        let launch = self.take_launch(pane, generation).filter(|_| !detaching);
        let (Some(launch), Some(adopter)) = (
            launch,
            self.adopter.as_ref().and_then(|adopter| adopter.detached()),
        ) else {
            if let Some(record) = record.filter(|_| !detaching)
                && child_runs_or_collect(&record)
            {
                self.notify_process(
                    NoticeLevel::Error,
                    Some(record.process().clone()),
                    with_pids(&format!("{ADOPT_FAILED_NOTICE}{LEFT_RUNNING}"), &record),
                );
            }
            return;
        };
        let LaunchPurpose::Adopt {
            carried: Some(carried),
        } = launch.purpose()
        else {
            return;
        };
        let record = record.filter(|record| {
            self.pane_named(record.process()) == Some(pane) && !self.holds_process(pane)
        });
        let Some(record) = record else {
            let membership = launch.membership();
            let intent = launch.intent();
            self.workspace.set_state(pane, launch.prior_state());
            self.reconnect_failed(pane, carried.with_membership(membership), None, None);
            if self.workspace.process(pane).is_some() {
                self.apply_pending(pane, membership, None, intent);
            }
            return;
        };
        let since = Some(self.output_position(pane).0);
        let project = self.root().clone();
        let events = self.events.clone();
        let sink = ChannelOutputSink::new(project.clone(), pane, generation, events.clone());
        self.launches.begin(pane, launch);
        thread::spawn(move || {
            let result = adopter.adopt(&record, since, Box::new(sink));
            let _ = events.send(ServiceEvent::Adopted {
                project,
                pane,
                generation,
                record: Box::new(record),
                result,
            });
        });
    }

    /// Tries again, as a start asks, to adopt the holder of `pane`'s child
    /// left running unsupervised: a worker looks for a holder serving it, and
    /// [`Self::handle_adoption_probed`] adopts it or says none does. Returns
    /// whether a try began, which it does not while the child is being
    /// stopped or no adopter is wired.
    pub(super) fn retry_adoption_now(&mut self, pane: PaneId) -> bool {
        if self.adopter.is_none() || self.retryable_pid(pane).is_none() {
            return false;
        }
        let attempt = self.adoption_retries.begin(pane);
        self.probe_adoption(pane, attempt, true);
        true
    }

    /// Looks again for the holder of `pane`'s unsupervised child once the
    /// backoff of try `attempt` has passed, unless a later try superseded it
    /// or the child is no longer left alone.
    pub fn handle_adoption_retry_due(&mut self, pane: PaneId, attempt: u64) {
        if self.adoption_retries.is_current(pane, attempt) && self.retryable_pid(pane).is_some() {
            self.probe_adoption(pane, attempt, false);
        }
    }

    /// Settles a worker's look for the holder of `pane`'s unsupervised child
    /// for try `attempt`: adopts `record` when a holder serves that child, and
    /// otherwise schedules the next timed try, telling a start that asked
    /// that the child still runs unsupervised. A superseded try, or a child
    /// no longer left alone, is ignored.
    pub fn handle_adoption_probed(
        &mut self,
        pane: PaneId,
        attempt: u64,
        asked: bool,
        record: Option<HolderRecord>,
    ) {
        if !self.adoption_retries.is_current(pane, attempt) {
            return;
        }
        let Some(pid) = self.retryable_pid(pane) else {
            return;
        };
        match record.filter(|record| record.child_pid() == pid) {
            Some(record) => {
                if let Some(target) = self.panes.get_mut(&pane) {
                    target.set_unsupervised(None);
                }
                self.begin_adoption(&record, None);
            },
            None => {
                if asked {
                    self.refuse_unsupervised_start(pane, pid);
                }
                self.schedule_adoption_retry(pane);
            },
        }
    }

    /// The pid of `pane`'s unsupervised child while it runs and is left
    /// alone: neither stopped nor being stopped.
    fn retryable_pid(&self, pane: PaneId) -> Option<Pid> {
        let left_alone = self
            .workspace
            .process(pane)
            .is_some_and(|process| *process.state() == ProcessState::Unsupervised);
        left_alone
            .then(|| self.running_unsupervised_pid(pane))
            .flatten()
    }

    /// Looks for a holder serving `pane`'s unsupervised child for try
    /// `attempt`, on a worker when the adopter allows it, since reading the
    /// holder records and probing a socket may block, and inline otherwise.
    fn probe_adoption(&mut self, pane: PaneId, attempt: u64, asked: bool) {
        let (Some(pid), Some(adopter)) = (self.retryable_pid(pane), self.adopter.as_ref()) else {
            return;
        };
        let Some(detached) = adopter.detached() else {
            let record = adopter.serving(pid);
            self.handle_adoption_probed(pane, attempt, asked, record);
            return;
        };
        let events = self.events.clone();
        let project = self.root().clone();
        thread::spawn(move || {
            let record = detached.serving(pid).map(Box::new);
            let _ = events.send(ServiceEvent::AdoptionProbed {
                project,
                pane,
                attempt,
                asked,
                record,
            });
        });
    }

    /// Schedules the next timed try at adopting the holder of `pane`'s
    /// unsupervised child, backing off from the last.
    fn schedule_adoption_retry(&mut self, pane: PaneId) {
        let (attempt, delay) = self.adoption_retries.schedule(pane);
        let events = self.events.clone();
        let project = self.root().clone();
        thread::spawn(move || {
            thread::sleep(delay);
            let _ = events.send(ServiceEvent::AdoptionRetryDue {
                project,
                pane,
                attempt,
            });
        });
    }

    /// Settles an adoption of `pane` that failed while its child runs on:
    /// tells why, once per holder however often a retry fails the same way,
    /// and schedules the next timed try.
    fn adoption_failed_unsupervised(
        &mut self,
        pane: PaneId,
        record: &HolderRecord,
        error: &PtyError,
    ) {
        if self
            .adoption_retries
            .first_failure(pane, record.holder_pid())
        {
            let notice = format!("{ADOPT_FAILED_NOTICE}{LEFT_RUNNING}: {error}");
            self.notify(NoticeLevel::Error, Some(pane), with_pids(&notice, record));
        }
        self.schedule_adoption_retry(pane);
    }

    /// Settles a reconnect that could not supervise the holder again: the
    /// child is left unsupervised while it runs, with a stop in flight carried
    /// over, and a gone child settles as a failed exit, its holder's exit
    /// collected. `record` is the holder's record when one was found and
    /// `error` why adopting it failed.
    fn reconnect_failed(
        &mut self,
        pane: PaneId,
        carried: CarriedPane,
        record: Option<&HolderRecord>,
        error: Option<PtyError>,
    ) {
        let start_token = record.and_then(HolderRecord::child_start_token);
        let running = carried
            .pid()
            .is_some_and(|pid| self.leave_unsupervised(pane, pid, start_token));
        if let (Some(record), false) = (record, running) {
            collect_exit(record);
        }
        match (record, error) {
            (Some(record), Some(error)) if running => {
                self.adoption_failed_unsupervised(pane, record, &error);
            },
            (Some(record), Some(error)) => {
                let notice = format!("{ADOPT_FAILED_NOTICE}{SINCE_EXITED}: {error}");
                self.notify(NoticeLevel::Error, Some(pane), with_pids(&notice, record));
            },
            (None, _) => {
                let pid = carried.pid().map(|pid| pid.to_string()).unwrap_or_default();
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
            self.continue_stop_unsupervised(pane, carried.intent());
            return;
        }
        if let Some(target) = self.panes.get_mut(&pane) {
            target.restore(carried);
        }
        self.apply_exit(pane, ExitOutcome::FAILED_UNKNOWN);
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
                policy.grace_period().into_inner(),
            );
        }
    }

    /// Starts adopting the holder `record` describes, as [`Self::adopt`] does,
    /// or reconnecting `reconnecting`'s pane to it with what that pane
    /// carries. The pane is `Starting` until the adoption settles; it runs on
    /// a worker when the adopter allows it, and inline otherwise.
    fn begin_adoption(
        &mut self,
        record: &HolderRecord,
        reconnecting: Option<(PaneId, CarriedPane)>,
    ) {
        let removed = reconnecting.is_none() && self.pane_named(record.process()).is_none();
        let Some(pane) = self.adoptable_pane(record) else {
            if let Some((pane, carried)) = reconnecting {
                self.bump_generation(pane);
                self.reconnect_failed(pane, carried, Some(record), None);
            }
            return;
        };
        if reconnecting.is_none() {
            self.launches
                .set_environment(pane, record.launch_environment().iter().cloned().collect());
        }
        let carried = reconnecting.map(|(_, carried)| carried);
        let generation = self.bump_generation(pane);
        let project = self.root().clone();
        let events = self.events.clone();
        let sink = ChannelOutputSink::new(project.clone(), pane, generation, events.clone());
        let Some(adopter) = self.adopter.as_ref() else {
            let running =
                self.leave_unsupervised(pane, record.child_pid(), record.child_start_token());
            if !running {
                collect_exit(record);
            }
            let notice = format!("{ADOPT_UNAVAILABLE_NOTICE}{}", outcome(running));
            self.notify(NoticeLevel::Error, Some(pane), with_pids(&notice, record));
            if removed {
                self.stop_removed(pane);
            }
            return;
        };
        let since = carried.is_some().then(|| self.output_position(pane).0);
        let adoption = match adopter.detached() {
            Some(detached) => Adoption::Detached(detached, sink),
            None => Adoption::Inline(adopter.adopt(record, since, Box::new(sink))),
        };
        let prior_state = self
            .workspace
            .process(pane)
            .map_or(ProcessState::Pending, |process| *process.state());
        self.launches.begin(
            pane,
            PendingLaunch::new(generation, prior_state, LaunchPurpose::Adopt { carried }),
        );
        self.workspace.set_state(pane, ProcessState::Starting);
        if removed {
            self.stop_removed(pane);
        }
        match adoption {
            Adoption::Detached(adopter, sink) => {
                let record = record.clone();
                thread::spawn(move || {
                    let result = adopter.adopt(&record, since, Box::new(sink));
                    let _ = events.send(ServiceEvent::Adopted {
                        project,
                        pane,
                        generation,
                        record: Box::new(record),
                        result,
                    });
                });
            },
            Adoption::Inline(result) => self.settle_adoption(pane, generation, record, result),
        }
    }

    /// Stops `pane`, the adopted process of a holder whose process is no longer
    /// in the project, with a notice: its row retires once the process exits,
    /// or at once when nothing runs. A pane still starting takes the stop once
    /// its adoption settles.
    fn stop_removed(&mut self, pane: PaneId) {
        self.notify(NoticeLevel::Warning, Some(pane), ADOPT_REMOVED_NOTICE);
        self.set_membership(pane, ConfigMembership::RetireOnExit);
        if self.launches.record_intent(pane, PendingIntent::Stop) {
            return;
        }
        if self.holds_process(pane) {
            self.stop_pane(pane);
        } else {
            self.retire_pane(pane);
        }
    }

    /// The pane of the process `record` runs, when it holds nothing yet. A
    /// process no longer in the project gets a pane of its own, built from the
    /// record, to be stopped from. A process already supervised is a second
    /// copy: it is left running and said so while its child runs, and its
    /// holder's exit is collected otherwise.
    fn adoptable_pane(&mut self, record: &HolderRecord) -> Option<PaneId> {
        let Some(pane) = self.pane_named(record.process()) else {
            return Some(self.add_removed_pane(record));
        };
        if self.holds_process(pane) {
            if child_runs_or_collect(record) {
                self.notify(
                    NoticeLevel::Warning,
                    Some(pane),
                    with_pids(ADOPT_DUPLICATE_NOTICE, record),
                );
            }
            return None;
        }
        Some(pane)
    }

    /// Adds a pane for the process `record` runs that is no longer in the
    /// project: its name, command and stop policy come from the record, and it
    /// never restarts or autostarts. Returns the pane, after every other.
    fn add_removed_pane(&mut self, record: &HolderRecord) -> PaneId {
        let pane = PaneId::new(
            self.workspace
                .processes()
                .iter()
                .map(|process| process.id().into_inner() + 1)
                .chain(self.generations.keys().map(|pane| pane.into_inner() + 1))
                .max()
                .unwrap_or_default(),
        );
        let spec = ProcessSpec::builder()
            .name(record.process().clone())
            .command(record.command().clone())
            .environment(Environment::empty())
            .autostart(false)
            .restart(RestartPolicy::Never)
            .stop(record.stop())
            .build();
        self.workspace.push(spec.to_process(pane));
        pane
    }

    /// Settles the adoption of `pane` at `generation` with `result`: the
    /// holder is supervised with the output it sent meanwhile, a reconnect
    /// gets back what it carried, and a failure leaves a running child
    /// unsupervised and has a gone child's exit collected. Then what was asked
    /// while it was starting applies: the membership a stack change gave it, a
    /// pause or resume when the adoption installed a live process, and a stop,
    /// kill or restart, which a child left unsupervised receives through the
    /// unsupervised path. A superseded result is never installed: it is
    /// detached when a shutdown superseded it and stopped otherwise, and a
    /// superseded failure whose child still runs is said so.
    fn settle_adoption(
        &mut self,
        pane: PaneId,
        generation: SpawnGeneration,
        record: &HolderRecord,
        result: Result<Box<dyn ProcessHandle>, PtyError>,
    ) {
        let Some(launch) = self.take_launch(pane, generation) else {
            let release = self.stale_release(pane, generation);
            match result {
                Ok(handle) => release.release(handle),
                Err(error) => self.report_superseded_failure(record, &error),
            }
            return;
        };
        let prior_state = launch.prior_state();
        let intent = launch.intent();
        let suspend = launch.suspend();
        let membership = launch.membership();
        let LaunchPurpose::Adopt { carried, .. } = launch.purpose() else {
            if let Ok(handle) = result {
                handle.abandon();
            }
            return;
        };
        let carried = carried.map(|carried| carried.with_membership(membership));
        let handle = match result {
            Ok(handle) => handle,
            Err(error) => {
                self.workspace.set_state(pane, prior_state);
                if let Some(carried) = carried {
                    self.reconnect_failed(pane, carried, Some(record), Some(error));
                    if self.workspace.process(pane).is_some() {
                        self.apply_pending(pane, membership, None, intent);
                    }
                    return;
                }
                if self.leave_unsupervised(pane, record.child_pid(), record.child_start_token()) {
                    self.adoption_failed_unsupervised(pane, record, &error);
                } else {
                    collect_exit(record);
                    let notice = format!("{ADOPT_FAILED_NOTICE}{SINCE_EXITED}: {error}");
                    self.notify(NoticeLevel::Error, Some(pane), with_pids(&notice, record));
                }
                self.apply_pending(pane, membership, None, intent);
                return;
            },
        };
        self.adoption_retries.forget(pane);
        let backlog = self.install_pane(pane, handle);
        if carried.is_none() {
            self.restarts.take_history(pane, record.history());
            self.adopt_run(pane);
        }
        self.arm_probes(pane);
        if let Some(carried) = carried {
            if let Some(target) = self.panes.get_mut(&pane) {
                target.restore(carried);
            }
            if let Some(state) = carried.state() {
                self.workspace.set_state(pane, state);
            }
            if carried.intent().awaits_force_stop() {
                self.rearm_force_stop(pane);
            }
        }
        self.replay(pane, generation, backlog, launch.into_early());
        let suspend = suspend.filter(|_| self.runs_launch(pane, generation));
        self.apply_pending(pane, membership, suspend, intent);
    }

    /// Reports an adoption that failed after a later action superseded it,
    /// when its child still runs, so a child is never dropped silently; a
    /// gone child's exit is collected instead.
    fn report_superseded_failure(&self, record: &HolderRecord, error: &PtyError) {
        if child_runs_or_collect(record) {
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
        ffi::OsString,
        path::{Path, PathBuf},
        process,
        rc::Rc,
        sync::atomic::{AtomicUsize, Ordering},
        time::{Duration, Instant, SystemTime},
    };

    use rustix::process::Signal;

    use super::*;
    use crate::{
        adapter::{
            holder::HolderId,
            service::{
                LaunchEnvironment, ServiceEvent,
                supervisor::{
                    ConfigMembership, SpawnGeneration,
                    lifecycle::FORCE_STOP_NOTICE,
                    testing::*,
                    unsupervised::{UNSUPERVISED_KILL_LIMIT, UNSUPERVISED_ROW_REMOVED},
                    unsupervised_child::UnsupervisedChild,
                },
            },
        },
        domain::{
            port::ProcessRunner,
            process::{ProcessState, RestartPolicy, StopPolicy, StopSignal},
            pty::{ExitOutcome, ProcessOutput},
            value::{CommandLine, ProcessName, ProjectRoot, StartToken, StopGrace},
        },
    };

    const PANE: u64 = 0;
    /// A pid that always runs and is never a holder, standing in for one.
    const KERNEL_THREAD_PID: u32 = 2;
    /// The command a stack change replaces the test command with.
    const OTHER_COMMAND: &str = "serve --again";
    /// A start token no running process has.
    const FOREIGN_TOKEN: u64 = 1;
    /// How long before the test a reconnected process claims to have started.
    const EARLIER: Duration = Duration::from_secs(3600);
    /// Longest a test waits for a scheduled respawn to arrive.
    const RESPAWN_WAIT: Duration = Duration::from_secs(5);
    /// A socket path nothing serves.
    const UNSERVED_SOCKET: &str = "/nonexistent/holder.sock";
    /// Grace the graceful stop policy gives before the kill.
    const STOP_GRACE: Duration = Duration::from_millis(300);
    /// A grace far longer than an early exit takes to be seen.
    const LONG_STOP_GRACE: Duration = Duration::from_secs(8);
    /// Within this a stopped child's early exit is seen, well inside
    /// [`LONG_STOP_GRACE`].
    const EARLY_EXIT_BOUND: Duration = Duration::from_secs(2);

    /// The test process's name.
    fn p() -> ProcessName {
        name("p")
    }

    /// No environment of the client's own.
    fn no_env() -> LaunchEnvironment {
        LaunchEnvironment::default()
    }

    /// The graceful stop policy of the adoption tests.
    fn graceful() -> StopPolicy {
        StopPolicy::builder()
            .signal(StopSignal::Interrupt)
            .grace_period(StopGrace::try_new(STOP_GRACE).unwrap())
            .build()
    }

    /// `harness` able to adopt through `adopter`.
    fn with_adopter(harness: Harness, adopter: Box<dyn HolderAdopter>) -> Harness {
        harness.adopting_through(adopter)
    }

    /// A harness over `p`, restarting always, that can adopt holders.
    fn adopting(runner: Box<dyn ProcessRunner>) -> Harness {
        with_adopter(
            Harness::over(runner, &[spec("p", "true", RestartPolicy::Always)]),
            Box::new(FakeAdopter),
        )
    }

    /// The record a holder running `process` of this project as `child_pid`
    /// would leave, serving at `socket`.
    fn record_at(process: &str, child_pid: Pid, socket: &Path) -> HolderRecord {
        HolderRecord::builder()
            .id(HolderId::generate().unwrap())
            .project(ProjectRoot::try_new(PathBuf::from(TEST_PROJECT)).unwrap())
            .process(name(process))
            .command(CommandLine::try_new("sleep 600").unwrap())
            .stop(graceful())
            .launch_environment(vec![(OsString::from("FROM"), OsString::from("cli"))])
            .socket(socket.to_path_buf())
            .holder_pid(Pid::try_new(KERNEL_THREAD_PID).unwrap())
            .child_pid(child_pid)
            .child_start_token(None)
            .log(PathBuf::from("/tmp/holder.log"))
            .child_started_at(SystemTime::now())
            .build()
    }

    /// As [`record_at`], at a socket nothing serves.
    fn record_with_pid(process: &str, child_pid: Pid) -> HolderRecord {
        record_at(process, child_pid, Path::new(UNSERVED_SOCKET))
    }

    /// The record of `process` running as the fake launcher pid.
    fn record(process: &str) -> HolderRecord {
        record_with_pid(process, fake_launcher_pid())
    }

    /// A pid that belonged to a process that has exited.
    fn gone_pid() -> Pid {
        let mut exited = process::Command::new("true").spawn().unwrap();
        let pid = Pid::try_new(exited.id()).unwrap();
        exited.wait().unwrap();
        pid
    }

    #[test]
    fn adopted_process_is_running_with_its_pid_and_restart_policy_applies_on_its_exit() {
        let spawns = SpawnRecorder::default();
        let mut harness = adopting(Box::new(
            RequestRecordingRunner::builder()
                .recorder(spawns.clone())
                .build(),
        ));
        let pane = PaneId::new(PANE);

        harness.supervisor_mut().adopt(record("p"));
        harness.up();

        let status = harness.supervisor_mut().snapshot();
        assert_eq!(*status.processes()[0].state(), ProcessState::Running);
        assert_eq!(*status.processes()[0].pid(), Some(fake_launcher_pid()));
        assert!(
            spawns.requests().borrow().is_empty(),
            "up leaves an adopted process running rather than starting another"
        );

        let generation = harness.supervisor_mut().generations()[&pane];
        harness.supervisor_mut().handle_output(
            pane,
            generation,
            ProcessOutput::Exited(ExitOutcome::FAILED_UNKNOWN),
        );

        assert_eq!(state(&harness), ProcessState::Restarting);
        let Ok(ServiceEvent::Respawn {
            pane: respawned,
            generation,
            ..
        }) = harness.events().recv_timeout(RESPAWN_WAIT)
        else {
            panic!("the restart policy schedules a respawn");
        };
        harness
            .supervisor_mut()
            .handle_respawn(respawned, generation);
        let requests = spawns.requests().borrow();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0]
                .launch_environment()
                .get(&OsString::from("FROM")),
            Some(&OsString::from("cli")),
            "the policy restart keeps the environment the holder was launched with"
        );
    }

    /// A holder whose process is no longer in the project is adopted to be
    /// stopped through the stop policy its record carries, with a notice, and
    /// its row goes once it exits.
    #[test]
    fn a_removed_process_found_running_is_stopped_with_a_notice_and_discarded() {
        let signals = Arc::new(StopSignals::default());
        let mut harness = with_adopter(
            Harness::over(Box::new(FakeRunner), &[spec(
                "p",
                "true",
                RestartPolicy::Never,
            )]),
            Box::new(SignalAdopter::new(Arc::clone(&signals))),
        );

        harness.supervisor_mut().adopt(record("gone"));

        let gone = harness
            .supervisor_mut()
            .workspace()
            .processes()
            .iter()
            .find(|process| process.name() == &name("gone"))
            .map(|process| *process.id())
            .expect("the removed process has a row while it stops");
        assert_eq!(
            *signals.termination().lock().unwrap(),
            Some((StopSignal::Interrupt, STOP_GRACE)),
            "stopped through the recorded stop policy"
        );
        assert_eq!(
            harness
                .notices()
                .lock()
                .unwrap()
                .recent(harness.supervisor().root())[0]
                .process(),
            &Some(name("gone"))
        );
        assert!(
            harness
                .notice_messages()
                .contains(&ADOPT_REMOVED_NOTICE.to_string())
        );
        let generation = harness.supervisor_mut().generations()[&gone];
        harness.supervisor_mut().handle_output(
            gone,
            generation,
            ProcessOutput::Exited(ExitOutcome::Succeeded),
        );
        assert!(harness.supervisor_mut().workspace().process(gone).is_none());
        assert_eq!(harness.supervisor_mut().process_names(), vec![p()]);
    }

    /// A record naming an already supervised process whose child is gone is
    /// not adoptable, so its holder's exit is collected rather than left held
    /// forever, and nothing says a child was left running.
    #[test]
    fn an_unadoptable_duplicate_whose_child_is_gone_has_its_exit_collected() {
        let holder = ExitedHolder::serve();
        let mut harness = adopting(Box::new(FakeRunner));
        harness.up();

        harness
            .supervisor_mut()
            .adopt(record_at("p", gone_pid(), holder.socket()));

        assert!(holder.was_collected());
        assert!(
            harness.notice_messages().is_empty(),
            "no left-running notice: {:?}",
            harness.notice_messages()
        );
    }

    /// An adoption that fails on a child that is gone collects its holder's
    /// exit and says the child exited, never that it was left running.
    #[test]
    fn a_failed_adoption_of_a_gone_child_collects_its_exit() {
        let holder = ExitedHolder::serve();
        let mut harness = with_adopter(
            Harness::over(Box::new(FakeRunner), &[spec(
                "p",
                "true",
                RestartPolicy::Never,
            )]),
            Box::new(
                ReconnectingAdopter::builder()
                    .record(None)
                    .refuses(true)
                    .adoptions(Rc::new(Cell::new(0)))
                    .build(),
            ),
        );

        harness
            .supervisor_mut()
            .adopt(record_at("p", gone_pid(), holder.socket()));

        assert!(holder.was_collected());
        let messages = harness.notice_messages();
        assert!(
            messages
                .iter()
                .any(|message| message.contains(SINCE_EXITED))
        );
        assert!(
            !messages
                .iter()
                .any(|message| message.contains(LEFT_RUNNING))
        );
        assert_eq!(state(&harness), ProcessState::Pending);
    }

    /// The test process `p`, restarting always and stopped gracefully,
    /// running `command`.
    fn graceful_spec(command: &str) -> ProcessSpec {
        spec("p", command, RestartPolicy::Always).with_stop(graceful())
    }

    /// A harness over `p` (graceful stop, restart `Always`) whose processes
    /// report `pid` and whose holder `adopter` can reconnect to, plus its
    /// spawn count. Nothing is started.
    fn reconnectable_over(
        pid: Pid,
        adopter: impl HolderAdopter + 'static,
    ) -> (Harness, Arc<AtomicUsize>) {
        let spawns = Arc::new(AtomicUsize::new(0));
        let harness = with_adopter(
            Harness::over(
                Box::new(
                    FixedPidRunner::builder()
                        .pid(pid)
                        .spawns(spawns.clone())
                        .build(),
                ),
                &[graceful_spec("true")],
            ),
            Box::new(adopter),
        );
        (harness, spawns)
    }

    /// As [`reconnectable_over`], started, with a pid no test relies on.
    fn reconnectable(adopter: ReconnectingAdopter) -> Harness {
        let (mut harness, _) = reconnectable_over(fake_launcher_pid(), adopter);
        harness.up();
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
            match harness.events().recv_timeout(remaining) {
                Ok(ServiceEvent::ForceStop {
                    pane,
                    spawn_generation,
                    shutdown_generation,
                    ..
                }) => harness.supervisor_mut().handle_force_stop(
                    pane,
                    spawn_generation,
                    shutdown_generation,
                ),
                Ok(_) => {},
                Err(_) => panic!("timed out waiting for {what}"),
            }
        }
    }

    /// A supervisor of `p` (graceful stop) left running unsupervised as
    /// `orphan` by an adoption that failed at startup, plus its spawn count.
    fn configured_unsupervised(orphan: &Orphan) -> (Harness, Arc<AtomicUsize>) {
        let (mut harness, spawns) = reconnectable_over(
            orphan.pid(),
            ReconnectingAdopter::builder()
                .record(None)
                .refuses(true)
                .adoptions(Rc::new(Cell::new(0)))
                .build(),
        );
        harness
            .supervisor_mut()
            .adopt(record_with_pid("p", orphan.pid()));
        assert_eq!(state(&harness), ProcessState::Unsupervised);
        (harness, spawns)
    }

    /// The shared assertions on an unsupervised `p` running as `orphan`: it is
    /// reported with its pid, refuses to start or restart a second copy, and a
    /// stop signals it but reports `Exited` only once the pid is gone.
    fn assert_unsupervised_until_gone(
        harness: &mut Harness,
        spawns: &AtomicUsize,
        orphan: &Orphan,
    ) {
        assert_eq!(state(harness), ProcessState::Unsupervised);
        assert!(!harness.supervisor_mut().is_live(PaneId::new(PANE)));
        assert_eq!(
            *harness.supervisor_mut().snapshot().processes()[0].pid(),
            Some(orphan.pid())
        );
        let spawned = spawns.load(Ordering::SeqCst);

        harness.supervisor_mut().start(&p(), &no_env());
        harness.supervisor_mut().restart(&p(), &no_env());

        assert_eq!(spawns.load(Ordering::SeqCst), spawned, "no second copy");
        assert_eq!(state(harness), ProcessState::Unsupervised);
        assert!(
            harness
                .notice_messages()
                .iter()
                .any(|message| message.ends_with(&orphan.pid().to_string()))
        );

        harness.supervisor_mut().stop(&p());
        assert_eq!(state(harness), ProcessState::Stopping);
        assert!(orphan.alive(), "the graceful signal is ignored");

        drive_until_exited(harness);
        assert!(!orphan.alive(), "exited only once the pid is gone");
    }

    /// The current generation of [`PANE`].
    fn generation(harness: &Harness) -> SpawnGeneration {
        harness.supervisor().generations()[&PaneId::new(PANE)]
    }

    /// The state of `p`.
    fn state(harness: &Harness) -> ProcessState {
        *harness
            .supervisor()
            .workspace()
            .process(PaneId::new(PANE))
            .unwrap()
            .state()
    }

    /// The state of the process on `pane`, while it has a row.
    fn state_of(harness: &Harness, pane: PaneId) -> Option<ProcessState> {
        harness
            .supervisor()
            .workspace()
            .process(pane)
            .map(|process| *process.state())
    }

    /// Reports `p`'s output stream as broken.
    fn break_stream(harness: &mut Harness) {
        let current = generation(harness);
        harness.supervisor_mut().handle_output(
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
        let mut harness = reconnectable(
            ReconnectingAdopter::builder()
                .record(Some(record("p")))
                .refuses(false)
                .adoptions(adoptions.clone())
                .build(),
        );
        let pane = PaneId::new(PANE);
        let broken = generation(&harness);

        break_stream(&mut harness);

        assert_eq!(adoptions.get(), 1);
        assert!(harness.supervisor_mut().is_live(pane));
        assert_eq!(state(&harness), ProcessState::Running);
        let current = generation(&harness);
        assert_ne!(current, broken);
        harness.supervisor_mut().handle_output(
            pane,
            broken,
            ProcessOutput::Chunk(b"stale".to_vec()),
        );
        harness.supervisor_mut().handle_output(
            pane,
            current,
            ProcessOutput::Chunk(b"fresh".to_vec()),
        );
        let tail = harness.run_tail(pane);
        assert!(tail.iter().any(|line| line.contains("fresh")), "{tail:?}");
        assert!(!tail.iter().any(|line| line.contains("stale")), "{tail:?}");

        harness.supervisor_mut().stop(&p());
        let current = generation(&harness);
        harness.supervisor_mut().handle_output(
            pane,
            current,
            ProcessOutput::Exited(ExitOutcome::FAILED_UNKNOWN),
        );
        assert_eq!(state(&harness), ProcessState::Exited);
        assert!(!harness.supervisor_mut().is_live(pane));
    }

    /// A graceful stop already delivered when the stream breaks carries over
    /// the reconnect, so the exit the holder reports on the new connection
    /// settles as a stop rather than a restart.
    #[test]
    fn a_stop_in_flight_survives_a_reconnect() {
        let mut harness = reconnectable(
            ReconnectingAdopter::builder()
                .record(Some(record("p")))
                .refuses(false)
                .adoptions(Rc::new(Cell::new(0)))
                .build(),
        );
        harness.supervisor_mut().stop(&p());
        assert_eq!(state(&harness), ProcessState::Stopping);

        break_stream(&mut harness);
        assert_eq!(state(&harness), ProcessState::Stopping);
        let current = generation(&harness);
        harness.supervisor_mut().handle_output(
            PaneId::new(PANE),
            current,
            ProcessOutput::Exited(ExitOutcome::FAILED_UNKNOWN),
        );

        assert_eq!(state(&harness), ProcessState::Exited);
        assert!(
            !matches!(
                harness.events().try_recv(),
                Ok(ServiceEvent::Respawn { .. })
            ),
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
            let record = record_with_pid("p", orphan.pid());
            let (mut harness, spawns) = reconnectable_over(
                orphan.pid(),
                ReconnectingAdopter::builder()
                    .record(refuses.then_some(record))
                    .refuses(refuses)
                    .adoptions(Rc::new(Cell::new(0)))
                    .build(),
            );
            harness.up();

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

    /// A harness of `p` left unsupervised as `orphan` by a failed adoption of
    /// a live holder that refuses `refusals` adoptions in all, plus its spawn
    /// count and the adoptions tried.
    fn refused_adoption(
        orphan: &Orphan,
        refusals: usize,
    ) -> (Harness, Arc<AtomicUsize>, Rc<Cell<usize>>) {
        let record = record_with_pid("p", orphan.pid());
        let attempts = Rc::new(Cell::new(0));
        let (mut harness, spawns) = reconnectable_over(
            orphan.pid(),
            RecoveringAdopter::builder()
                .record(record.clone())
                .refusals(refusals)
                .attempts(Rc::clone(&attempts))
                .build(),
        );
        harness.supervisor_mut().adopt(record);
        assert_eq!(state(&harness), ProcessState::Unsupervised);
        (harness, spawns, attempts)
    }

    /// Lets the timed retry of `p`'s adoption come due now.
    fn retry_due(harness: &mut Harness) {
        let pane = PaneId::new(PANE);
        let attempt = harness
            .supervisor()
            .adoption_retries
            .current(pane)
            .expect("a retry is scheduled");
        harness
            .supervisor_mut()
            .handle_adoption_retry_due(pane, attempt);
    }

    /// A child left unsupervised by a failed adoption is adopted again once
    /// its holder serves: its timed retry does it, and so does a start, which
    /// then launches no second copy. A `ps` never retries.
    #[test]
    fn a_failed_adoption_is_retried_once_the_holder_serves() {
        for retry_by_start in [false, true] {
            let orphan = Orphan::spawn();
            let (mut harness, spawns, attempts) = refused_adoption(&orphan, 1);

            harness.supervisor_mut().snapshot();
            assert_eq!(attempts.get(), 1, "a ps retries nothing");
            let stale = harness
                .supervisor()
                .adoption_retries
                .current(PaneId::new(PANE))
                .unwrap();
            if retry_by_start {
                harness.supervisor_mut().start(&p(), &no_env());
            } else {
                retry_due(&mut harness);
            }

            assert_eq!(state(&harness), ProcessState::Running, "{retry_by_start}");
            assert_eq!(attempts.get(), 2);
            assert_eq!(spawns.load(Ordering::SeqCst), 0, "no second copy");
            harness
                .supervisor_mut()
                .handle_adoption_retry_due(PaneId::new(PANE), stale);
            assert_eq!(attempts.get(), 2, "a supervised child is left alone");
        }
    }

    /// A live holder that never lets itself be adopted leaves its child
    /// unsupervised after every retry, and is told once, not once a retry.
    #[test]
    fn a_holder_that_keeps_refusing_is_told_once_and_stays_unsupervised() {
        let orphan = Orphan::spawn();
        let (mut harness, _, attempts) = refused_adoption(&orphan, usize::MAX);

        retry_due(&mut harness);
        retry_due(&mut harness);

        assert_eq!(attempts.get(), 3);
        assert_eq!(state(&harness), ProcessState::Unsupervised);
        let told = harness
            .notice_messages()
            .iter()
            .filter(|message| message.starts_with(ADOPT_FAILED_NOTICE))
            .count();
        assert_eq!(told, 1, "{:?}", harness.notice_messages());
    }

    /// A stop of a child left unsupervised by a holder that refuses adoption
    /// is never undone by a retry coming due: the retry leaves it alone and
    /// the stop escalates to a kill that ends the child.
    #[test]
    fn a_stop_of_an_unsupervised_child_survives_a_due_retry() {
        let orphan = Orphan::spawn();
        let (mut harness, _, attempts) = refused_adoption(&orphan, usize::MAX);

        harness.supervisor_mut().stop(&p());
        retry_due(&mut harness);

        assert_eq!(attempts.get(), 1, "a retry never interrupts a stop");
        assert_eq!(state(&harness), ProcessState::Stopping);
        drive_until_exited(&mut harness);
        assert!(!orphan.alive(), "the stop killed the child");
    }

    /// A holder that cannot be adopted at startup, while its child runs, is
    /// unsupervised exactly as a failed reconnect is, and `up` starts no second
    /// copy beside it.
    #[test]
    fn a_holder_unadoptable_at_startup_is_unsupervised_until_its_pid_is_gone() {
        let orphan = Orphan::spawn();
        let (mut harness, spawns) = configured_unsupervised(&orphan);
        harness.up();

        assert_eq!(spawns.load(Ordering::SeqCst), 0, "up starts no second copy");
        assert_unsupervised_until_gone(&mut harness, &spawns, &orphan);
    }

    /// A failed reconnect whose child is verifiably gone is said so and
    /// settles as a failed exit: the restart policy applies, here a respawn.
    #[test]
    fn a_failed_reconnect_of_a_gone_child_settles_as_an_exit() {
        let pid = gone_pid();
        let (mut harness, spawns) = reconnectable_over(
            pid,
            ReconnectingAdopter::builder()
                .record(None)
                .refuses(false)
                .adoptions(Rc::new(Cell::new(0)))
                .build(),
        );
        harness.up();

        break_stream(&mut harness);

        assert_eq!(state(&harness), ProcessState::Restarting);
        assert!(harness.notice_messages().iter().any(|message| {
            message.starts_with(ADOPT_FAILED_NOTICE)
                && message.contains(SINCE_EXITED)
                && message.ends_with(&pid.to_string())
        }));
        let respawn = loop {
            match harness.events().recv_timeout(RESPAWN_WAIT) {
                Ok(ServiceEvent::Respawn {
                    pane, generation, ..
                }) => break (pane, generation),
                Ok(_) => {},
                Err(_) => panic!("the restart policy schedules a respawn"),
            }
        };
        harness
            .supervisor_mut()
            .handle_respawn(respawn.0, respawn.1);
        assert_eq!(spawns.load(Ordering::SeqCst), 2);
    }

    /// A reconnect keeps what outlives a connection: a process removed from
    /// the project still retires on its exit instead of respawning, and its
    /// uptime still counts from its real start.
    #[test]
    fn a_reconnect_keeps_stack_membership_and_start_time() {
        let mut harness = reconnectable(
            ReconnectingAdopter::builder()
                .record(Some(record("p")))
                .refuses(false)
                .adoptions(Rc::new(Cell::new(0)))
                .build(),
        );
        let pane = PaneId::new(PANE);
        let started_at = Instant::now() - EARLIER;
        if let Some(target) = harness.supervisor_mut().panes.get_mut(&pane) {
            target
                .set_config_membership(ConfigMembership::RetireOnExit)
                .set_started_at(started_at);
        }

        break_stream(&mut harness);

        let target = &harness.supervisor_mut().panes()[&pane];
        assert_eq!(target.config_membership(), ConfigMembership::RetireOnExit);
        assert!(
            harness.supervisor_mut().snapshot().processes()[0]
                .uptime()
                .unwrap()
                >= EARLIER
        );
        let current = generation(&harness);
        harness.supervisor_mut().handle_output(
            pane,
            current,
            ProcessOutput::Exited(ExitOutcome::FAILED_UNKNOWN),
        );
        assert!(harness.supervisor_mut().workspace().process(pane).is_none());
    }

    /// A paused process reconnects paused, so a resume still reaches it.
    #[test]
    fn a_paused_process_reconnects_paused_and_resumes() {
        let mut harness = reconnectable(
            ReconnectingAdopter::builder()
                .record(Some(record("p")))
                .refuses(false)
                .adoptions(Rc::new(Cell::new(0)))
                .build(),
        );
        harness.supervisor_mut().pause(&p());
        assert_eq!(state(&harness), ProcessState::Paused);

        break_stream(&mut harness);
        assert_eq!(state(&harness), ProcessState::Paused);

        harness.supervisor_mut().resume(&p());
        assert_eq!(state(&harness), ProcessState::Running);
    }

    /// A changed command updates the row in place, which keeps the
    /// unsupervised pid until the child is gone, and no start puts a second
    /// copy beside it meanwhile; once it is gone the new command starts.
    #[test]
    fn a_changed_command_keeps_the_unsupervised_pid_until_the_child_is_gone() {
        let orphan = Orphan::spawn();
        let (mut harness, spawns) = configured_unsupervised(&orphan);

        harness
            .supervisor_mut()
            .apply_stack(&stack(&[graceful_spec(OTHER_COMMAND)]));

        assert_eq!(
            *harness.supervisor_mut().snapshot().processes()[0].pid(),
            Some(orphan.pid())
        );
        harness.supervisor_mut().start(&p(), &no_env());
        harness.up();
        assert_eq!(spawns.load(Ordering::SeqCst), 0, "no second copy");

        harness.supervisor_mut().stop(&p());
        drive_until_exited(&mut harness);
        harness.supervisor_mut().start(&p(), &no_env());
        assert_eq!(spawns.load(Ordering::SeqCst), 1, "the new command starts");
    }

    /// An unsupervised child that exits on its own no longer blocks a start.
    #[test]
    fn an_orphan_that_exits_on_its_own_no_longer_blocks_a_start() {
        let orphan = Orphan::spawn();
        let (mut harness, spawns) = configured_unsupervised(&orphan);
        assert_eq!(orphan.end_with(Signal::KILL), Some(Signal::KILL.as_raw()));

        assert_eq!(
            *harness.supervisor_mut().snapshot().processes()[0].pid(),
            None
        );
        harness.supervisor_mut().start(&p(), &no_env());

        assert_eq!(spawns.load(Ordering::SeqCst), 1);
        assert_eq!(state(&harness), ProcessState::Running);
    }

    /// A pid now held by a process with another start token is not the
    /// unsupervised child: a stop settles it as gone and never signals it.
    #[test]
    fn a_reused_pid_is_never_signalled() {
        let orphan = Orphan::spawn();
        let (mut harness, _) = configured_unsupervised(&orphan);
        if let Some(target) = harness.supervisor_mut().panes.get_mut(&PaneId::new(PANE)) {
            target.set_unsupervised(Some(
                UnsupervisedChild::builder()
                    .pid(orphan.pid())
                    .start_token(Some(StartToken::try_new(FOREIGN_TOKEN).unwrap()))
                    .build(),
            ));
        }

        harness.supervisor_mut().force_kill(&p());

        assert_eq!(state(&harness), ProcessState::Exited);
        assert_eq!(
            orphan.end_with(Signal::USR1),
            Some(Signal::USR1.as_raw()),
            "the stop sent the reused pid nothing"
        );
    }

    /// A stop in flight when the stream breaks gets a fresh force-stop
    /// deadline on the new connection, so a child ignoring the stop signal is
    /// still killed rather than left `Stopping`.
    #[test]
    fn a_reconnect_rearms_the_deadline_of_a_stop_in_flight() {
        let mut harness = reconnectable(
            ReconnectingAdopter::builder()
                .record(Some(record("p")))
                .refuses(false)
                .adoptions(Rc::new(Cell::new(0)))
                .build(),
        );
        harness.supervisor_mut().stop(&p());

        break_stream(&mut harness);

        drive_until(&mut harness, "the re-armed force stop", |harness| {
            harness
                .notice_messages()
                .iter()
                .any(|message| message == FORCE_STOP_NOTICE)
        });
    }

    /// A child that survives every kill, standing in for one stuck in the
    /// kernel, ends the stop after the kill limit: an error names it and the
    /// process returns to `Unsupervised` with the stop retryable.
    #[test]
    fn a_child_that_survives_every_kill_leaves_the_stop_retryable() {
        let orphan = Orphan::spawn();
        let (mut harness, _) = configured_unsupervised(&orphan);

        harness.supervisor_mut().stop(&p());
        let child = harness
            .supervisor_mut()
            .panes_mut()
            .get_mut(&PaneId::new(PANE))
            .and_then(|target| target.unsupervised_mut().as_mut())
            .unwrap();
        child.end_grace();
        for _ in 0..UNSUPERVISED_KILL_LIMIT {
            child.count_kill();
        }
        drive_until(&mut harness, "the kills to give up", |harness| {
            state(harness) == ProcessState::Unsupervised
        });
        assert!(orphan.alive(), "no kill was sent past the limit");

        assert!(
            harness
                .notice_messages()
                .iter()
                .any(|message| message.ends_with(&orphan.pid().to_string()))
        );
        harness.supervisor_mut().stop(&p());
        assert_eq!(
            state(&harness),
            ProcessState::Stopping,
            "the stop is retried"
        );
    }

    /// A stopped unsupervised child that exits early in its grace is settled
    /// within a poll, not once the whole grace has passed.
    #[test]
    fn an_unsupervised_stop_finishes_once_the_child_is_gone() {
        let orphan = Orphan::spawn();
        let long_grace = StopPolicy::builder()
            .signal(StopSignal::Terminate)
            .grace_period(StopGrace::try_new(LONG_STOP_GRACE).unwrap())
            .build();
        let mut harness = with_adopter(
            Harness::over(
                Box::new(
                    FixedPidRunner::builder()
                        .pid(orphan.pid())
                        .spawns(Arc::new(AtomicUsize::new(0)))
                        .build(),
                ),
                &[spec("p", "true", RestartPolicy::Never).with_stop(long_grace)],
            ),
            Box::new(
                ReconnectingAdopter::builder()
                    .record(None)
                    .refuses(true)
                    .adoptions(Rc::new(Cell::new(0)))
                    .build(),
            ),
        );
        harness
            .supervisor_mut()
            .adopt(record_with_pid("p", orphan.pid()));
        assert_eq!(state(&harness), ProcessState::Unsupervised);

        harness.supervisor_mut().stop(&p());
        assert_eq!(state(&harness), ProcessState::Stopping);
        let stopped = Instant::now();
        assert_eq!(orphan.end_with(Signal::KILL), Some(Signal::KILL.as_raw()));
        drive_until_exited(&mut harness);

        assert!(
            stopped.elapsed() < EARLY_EXIT_BOUND,
            "settled after {:?}",
            stopped.elapsed()
        );
    }

    /// `ps` never shows an unsupervised process with no pid: a child found
    /// gone shows as exited.
    #[test]
    fn a_gone_unsupervised_child_shows_as_exited() {
        let orphan = Orphan::spawn();
        let (harness, _) = configured_unsupervised(&orphan);
        assert_eq!(orphan.end_with(Signal::KILL), Some(Signal::KILL.as_raw()));

        let status = harness.supervisor().snapshot();
        assert_eq!(*status.processes()[0].state(), ProcessState::Exited);
        assert_eq!(*status.processes()[0].pid(), None);
    }

    /// Removing an unsupervised process from the project stops its child, and
    /// its row goes once the child is gone.
    #[test]
    fn a_removed_unsupervised_row_goes_once_its_child_is_gone() {
        let orphan = Orphan::spawn();
        let (mut harness, _) = configured_unsupervised(&orphan);

        harness.supervisor_mut().apply_stack(&stack(&[]));
        assert_eq!(state(&harness), ProcessState::Stopping);

        drive_until(&mut harness, "the removed row to retire", |harness| {
            harness.supervisor().workspace().is_empty()
        });
        assert!(!orphan.alive());
    }

    /// Starting a row already removed from the project whose unsupervised
    /// child has since exited says the row was removed, and starts nothing.
    #[test]
    fn starting_a_removed_row_whose_child_is_gone_says_it_was_removed() {
        let orphan = Orphan::spawn();
        let (mut harness, spawns) = configured_unsupervised(&orphan);
        let old = PaneId::new(PANE);
        harness.supervisor_mut().apply_stack(&stack(&[]));
        assert_eq!(orphan.end_with(Signal::KILL), Some(Signal::KILL.as_raw()));

        harness.supervisor_mut().start(&p(), &no_env());

        assert!(harness.supervisor_mut().workspace().process(old).is_none());
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
            stranger.pid(),
            ReconnectingAdopter::builder()
                .record(None)
                .refuses(true)
                .adoptions(Rc::new(Cell::new(0)))
                .build(),
        );
        let record = HolderRecord::builder()
            .id(HolderId::generate().unwrap())
            .project(ProjectRoot::try_new(PathBuf::from(TEST_PROJECT)).unwrap())
            .process(p())
            .command(CommandLine::try_new("true").unwrap())
            .stop(graceful())
            .socket(PathBuf::from(UNSERVED_SOCKET))
            .holder_pid(Pid::try_new(KERNEL_THREAD_PID).unwrap())
            .child_pid(stranger.pid())
            .child_start_token(Some(StartToken::try_new(FOREIGN_TOKEN).unwrap()))
            .log(PathBuf::from("/tmp/holder.log"))
            .child_started_at(SystemTime::now())
            .build();

        harness.supervisor_mut().adopt(record);

        assert_ne!(state(&harness), ProcessState::Unsupervised);
        assert!(
            harness
                .notice_messages()
                .iter()
                .any(|message| message.contains(SINCE_EXITED))
        );
        harness.supervisor_mut().start(&p(), &no_env());
        assert_eq!(spawns.load(Ordering::SeqCst), 1);
        assert!(stranger.alive(), "the stranger is never signalled");
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
            match harness.events().recv_timeout(remaining) {
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

    /// Hands the supervisor of `harness` the holder a worker looked for after
    /// a stream broke, failing the test after [`SETTLE_WAIT`].
    fn located(harness: &mut Harness) {
        let deadline = Instant::now() + SETTLE_WAIT;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match harness.events().recv_timeout(remaining) {
                Ok(ServiceEvent::ReconnectLocated {
                    pane,
                    generation,
                    record,
                    ..
                }) => {
                    harness.supervisor_mut().handle_reconnect_located(
                        pane,
                        generation,
                        record.map(|record| *record),
                    );
                    return;
                },
                Ok(_) => {},
                Err(_) => panic!("the worker posts the holder it looked for"),
            }
        }
    }

    /// Looking for the holder of a broken stream runs on a worker: the
    /// supervisor answers at once while the look blocks, keeps a stop asked
    /// meanwhile, and applies it once the holder is adopted again.
    #[test]
    fn a_reconnect_looks_for_its_holder_off_the_loop() {
        let pid = fake_launcher_pid();
        let (open, gate) = crossbeam_channel::unbounded();
        let adoptions = Arc::new(AtomicUsize::new(0));
        let (mut harness, _) = reconnectable_over(
            pid,
            GatedLocateAdopter::builder()
                .record(record_with_pid("p", pid))
                .gate(gate)
                .adoptions(adoptions.clone())
                .build(),
        );
        harness.up();

        let broke = Instant::now();
        break_stream(&mut harness);
        assert!(
            broke.elapsed() < LOCATE_GATE_LIMIT,
            "the reconnect waited for the look"
        );
        assert_eq!(state(&harness), ProcessState::Starting);
        harness.supervisor_mut().stop(&p());
        assert_eq!(adoptions.load(Ordering::SeqCst), 0);

        open.send(()).unwrap();
        located(&mut harness);
        let (pane, generation, record, result) = adopted(&harness);
        harness
            .supervisor_mut()
            .handle_adopted(pane, generation, &record, result);

        assert_eq!(adoptions.load(Ordering::SeqCst), 1);
        assert_eq!(state(&harness), ProcessState::Stopping);
    }

    /// A removed process whose adoption fails on a worker while its child runs
    /// is still stopped: the child is left unsupervised, signalled, killed
    /// once it ignores the signal, and its row retires once it is gone.
    #[test]
    fn a_removed_process_whose_adoption_fails_is_killed_and_its_row_retires() {
        let orphan = Orphan::spawn();
        let mut harness = with_adopter(
            Harness::over(Box::new(FakeRunner), &[]),
            Box::new(FailingWorkerAdopter::builder().record(None).build()),
        );
        harness
            .supervisor_mut()
            .adopt(record_with_pid("gone", orphan.pid()));
        let pane = PaneId::new(PANE);
        assert_eq!(state_of(&harness, pane), Some(ProcessState::Starting));

        let (pane, generation, record, result) = adopted(&harness);
        harness
            .supervisor_mut()
            .handle_adopted(pane, generation, &record, result);

        drive_until(&mut harness, "the removed row to retire", |harness| {
            harness.supervisor().workspace().process(pane).is_none()
        });
        assert!(!orphan.alive(), "the child was killed");
    }

    /// A restart asked for while a reconnect was in flight is applied when
    /// the reconnect fails on a child that has exited and settles that exit.
    #[test]
    fn a_restart_kept_through_a_failed_reconnect_is_applied() {
        let pid = gone_pid();
        let (mut harness, spawns) = reconnectable_over(
            pid,
            ReconnectingAdopter::builder()
                .record(None)
                .refuses(false)
                .adoptions(Rc::new(Cell::new(0)))
                .build(),
        );
        harness.supervisor_mut().adopter = Some(Box::new(
            FailingWorkerAdopter::builder()
                .record(Some(record_with_pid("p", pid)))
                .build(),
        ));
        harness.up();
        assert_eq!(spawns.load(Ordering::SeqCst), 1);

        break_stream(&mut harness);
        assert_eq!(state(&harness), ProcessState::Starting);
        harness.supervisor_mut().restart(&p(), &no_env());
        located(&mut harness);
        let (pane, generation, record, result) = adopted(&harness);
        harness
            .supervisor_mut()
            .handle_adopted(pane, generation, &record, result);

        assert_eq!(spawns.load(Ordering::SeqCst), 2, "the kept restart ran");
        assert_eq!(state_of(&harness, pane), Some(ProcessState::Running));
    }

    /// A pause asked for while a reconnect was in flight belongs to the
    /// process that reconnect was for: when it fails and the restart already
    /// in flight launches a fresh process, that process is not paused.
    #[test]
    fn a_pause_kept_through_a_failed_reconnect_never_reaches_the_relaunched_process() {
        let pid = gone_pid();
        let (mut harness, spawns) = reconnectable_over(
            pid,
            ReconnectingAdopter::builder()
                .record(None)
                .refuses(false)
                .adoptions(Rc::new(Cell::new(0)))
                .build(),
        );
        harness.supervisor_mut().adopter = Some(Box::new(
            FailingWorkerAdopter::builder()
                .record(Some(record_with_pid("p", pid)))
                .build(),
        ));
        harness.up();
        harness.supervisor_mut().restart(&p(), &no_env());

        break_stream(&mut harness);
        assert_eq!(state(&harness), ProcessState::Starting);
        harness.supervisor_mut().pause(&p());
        located(&mut harness);
        let (pane, generation, record, result) = adopted(&harness);
        harness
            .supervisor_mut()
            .handle_adopted(pane, generation, &record, result);

        assert_eq!(
            spawns.load(Ordering::SeqCst),
            2,
            "the restart in flight ran"
        );
        assert_eq!(state_of(&harness, pane), Some(ProcessState::Running));
    }

    /// A stop in flight when the stream breaks, whose holder cannot be reached
    /// again while the child still runs, carries on against the child
    /// directly instead of being dropped: the process stays `Stopping` and
    /// settles once the child is gone.
    #[test]
    fn a_stop_in_flight_continues_when_the_reconnect_fails_while_the_child_runs() {
        let orphan = Orphan::spawn();
        let (mut harness, _) = reconnectable_over(
            orphan.pid(),
            ReconnectingAdopter::builder()
                .record(None)
                .refuses(false)
                .adoptions(Rc::new(Cell::new(0)))
                .build(),
        );
        harness.up();
        harness.supervisor_mut().stop(&p());
        assert_eq!(state(&harness), ProcessState::Stopping);

        break_stream(&mut harness);

        assert_eq!(state(&harness), ProcessState::Stopping);
        drive_until_exited(&mut harness);
        assert!(!orphan.alive());
    }
}
