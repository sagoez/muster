use std::{
    collections::HashMap,
    sync::PoisonError,
    time::{Duration, Instant, SystemTime},
};

use crossbeam_channel::Sender;

use super::{
    entity::Supervisor, run_end::RunEnd, run_readiness::RunReadiness,
    spawn_generation::SpawnGeneration,
};
use crate::{
    adapter::{
        holder::LineMatcher,
        service::{
            EventDraft, EventKind, ExitReason, LaunchEnvironment, MatchedLine, NoticeLevel,
            OutputTap, ProbeTarget, RunPin, ServiceErrorBody, ServiceErrorKind, ServiceResponse,
            TapLine, WaitCondition, WaitGroup, WaitOutcome, WaitReply, WaitSpec, WaitStatus,
            Waiter,
        },
    },
    domain::{
        port::ListeningSocket,
        process::{ProcessState, Readiness, ReadyCheck, ReadyProbe},
        value::{OutputCursor, PaneId, ProcessName},
    },
};

/// Lines of a run a wait that did not match carries.
pub const FAILURE_TAIL_LINES: usize = 30;
/// Shown when a readiness wait names a process with no readiness check.
const NO_READY_CHECK: &str = "has no readiness check; wait for a log line, port, URL or state";
/// Shown when a process name matches nothing in its project.
const UNKNOWN_PROCESS: &str = "no process with that name in this project";
/// Prefix of the notice for a stored readiness pattern that does not compile.
const READY_PATTERN_NOTICE: &str = "its readiness check cannot be used";

/// A wait checked against its process and ready to register: what it waits
/// for, resolved and compiled. Made by [`Supervisor::prepare_wait`] before a
/// request acts, so a bad wait refuses the request before anything changes.
pub struct PreparedWait {
    /// The pane of the process.
    pane: PaneId,
    /// The process.
    process: ProcessName,
    /// What it waits for.
    until: WaitCondition,
    /// The compiled pattern of a log condition.
    matcher: Option<LineMatcher>,
    /// Output after this cursor counts.
    since: Option<OutputCursor>,
    /// How long it waits.
    timeout: Duration,
}

impl Supervisor {
    /// Checks a wait on `name` as `spec` asks: resolves the default
    /// condition and compiles a log pattern.
    ///
    /// # Errors
    /// Returns the refusal for an unknown process, a pattern that does not
    /// compile, or a readiness wait on a process with no check.
    pub fn prepare_wait(
        &self,
        name: &ProcessName,
        spec: &WaitSpec,
    ) -> Result<PreparedWait, ServiceErrorBody> {
        let process = self
            .pane_named(name)
            .and_then(|pane| self.workspace.process(pane))
            .ok_or_else(|| {
                ServiceErrorBody::builder()
                    .kind(ServiceErrorKind::UnknownProcess)
                    .message(format!("{UNKNOWN_PROCESS}: {name}"))
                    .candidates(self.process_names())
                    .build()
            })?;
        let has_check = process.ready().is_some();
        let until = spec
            .until()
            .clone()
            .unwrap_or_else(|| WaitCondition::default_for(has_check));
        if until == WaitCondition::Ready && !has_check {
            return Err(invalid_input(format!("{name} {NO_READY_CHECK}")));
        }
        let matcher = match &until {
            WaitCondition::Log(pattern) => Some(
                LineMatcher::compile(pattern).map_err(|error| invalid_input(error.to_string()))?,
            ),
            _ => None,
        };
        Ok(PreparedWait {
            pane: *process.id(),
            process: process.name().clone(),
            until,
            matcher,
            since: *spec.since(),
            timeout: *spec.timeout(),
        })
    }

    /// The current spawn generation of the process `name`, so a wait
    /// registered after a start or restart can be pinned to the runs after it.
    pub fn generation_of(&self, name: &ProcessName) -> SpawnGeneration {
        self.pane_named(name)
            .map_or_else(SpawnGeneration::initial, |pane| {
                self.current_generation(pane)
            })
    }

    /// Whether the process `name` runs or is being started.
    pub fn runs_or_starts(&self, name: &ProcessName) -> bool {
        self.pane_named(name)
            .is_some_and(|pane| self.is_live(pane) || self.is_starting(pane))
    }

    /// Registers `prepared`, answering on `reply` when its condition holds,
    /// its run ends or its time runs out; a wait whose answer is already known
    /// is answered at once. With `after`, only runs started after that
    /// generation count, as for a wait that rides on a start or restart.
    pub fn begin_wait(
        &mut self,
        prepared: PreparedWait,
        after: Option<SpawnGeneration>,
        reply: Sender<ServiceResponse>,
    ) {
        self.register(prepared, after, WaitReply::Direct(reply));
    }

    /// Starts every autostart process that is not running, like
    /// [`Self::up`], then waits on each one it started for `spec`'s condition
    /// or that process's default, and answers the batch on `reply` once every
    /// wait ended.
    ///
    /// # Errors
    /// Returns the refusal, before anything starts, when a process that may
    /// start cannot take the wait.
    pub fn up_waiting(
        &mut self,
        environment: &LaunchEnvironment,
        spec: &WaitSpec,
        reply: Sender<ServiceResponse>,
    ) -> Result<(), ServiceErrorBody> {
        let mut prepared: HashMap<PaneId, (PreparedWait, SpawnGeneration)> = HashMap::new();
        for name in self.process_names() {
            let pane = self.pane_named(&name);
            let candidate = pane.is_some_and(|pane| {
                self.workspace
                    .process(pane)
                    .is_some_and(|process| *process.autostart())
                    && !self.is_live(pane)
            });
            if let (true, Some(pane)) = (candidate, pane) {
                let wait = self.prepare_wait(&name, spec)?;
                prepared.insert(pane, (wait, self.current_generation(pane)));
            }
        }
        let batch = self.up(environment);
        let acted: Vec<(PreparedWait, SpawnGeneration)> = batch
            .acted()
            .iter()
            .filter_map(|name| self.pane_named(name))
            .filter_map(|pane| prepared.remove(&pane))
            .collect();
        if acted.is_empty() {
            let _ = reply.send(ServiceResponse::Batch(batch));
            return Ok(());
        }
        let group = self.next_wait_group;
        self.next_wait_group += 1;
        self.wait_groups
            .insert(group, WaitGroup::new(batch, acted.len(), reply));
        for (slot, (wait, before)) in acted.into_iter().enumerate() {
            self.register(wait, Some(before), WaitReply::Batch { group, slot });
        }
        Ok(())
    }

    /// When the next wait or readiness check times out.
    pub fn next_wait_deadline(&self) -> Option<Instant> {
        let waits = self.waiters.iter().map(Waiter::deadline);
        let checks = self
            .readiness
            .values()
            .filter(|readiness| readiness.state.is_waiting())
            .map(|readiness| readiness.deadline);
        waits.chain(checks).min()
    }

    /// Times out the readiness checks and waits whose deadline passed, and
    /// answers every wait whose state or readiness condition now holds. The
    /// host runs it after each event it hands the supervisor.
    pub fn settle_waits(&mut self, now: Instant) {
        let expired: Vec<PaneId> = self
            .readiness
            .iter()
            .filter(|(_, readiness)| readiness.state.is_waiting() && readiness.deadline <= now)
            .map(|(pane, _)| *pane)
            .collect();
        for pane in expired {
            self.time_out_readiness(pane);
        }
        let mut index = 0;
        while index < self.waiters.len() {
            let waiter = &self.waiters[index];
            let settled = self
                .check(waiter)
                .or_else(|| (waiter.deadline() <= now).then_some((WaitStatus::Waiting, None)));
            match settled {
                Some((status, matched)) => {
                    let waiter = self.waiters.remove(index);
                    self.answer(waiter, status, matched);
                },
                None => index += 1,
            }
        }
    }

    /// Applies a passed port or HTTP probe to `pane`'s current run: passes
    /// its readiness check and answers the waits on that probe. A pass for a
    /// run that is no longer current is ignored.
    pub fn handle_probe_passed(
        &mut self,
        pane: PaneId,
        generation: SpawnGeneration,
        probe: &ProbeTarget,
    ) {
        if self.generations.get(&pane) != Some(&generation) {
            return;
        }
        if self.readiness.get(&pane).is_some_and(|readiness| {
            readiness.state.is_waiting() && readiness.probe().as_ref() == Some(probe)
        }) {
            self.pass_readiness(pane);
        }
        let mut index = 0;
        while index < self.waiters.len() {
            let waiter = &self.waiters[index];
            if waiter.pane() == pane
                && waiter.pin().counts(generation)
                && probe_of(waiter.until()).as_ref() == Some(probe)
            {
                let waiter = self.waiters.remove(index);
                self.answer(waiter, WaitStatus::Matched, None);
            } else {
                index += 1;
            }
        }
    }

    /// Records the ports each live process tree listens on, from a sample
    /// keyed by root pid; a process missing from it listens on none.
    pub fn take_ports(&mut self, sample: &HashMap<u32, Vec<ListeningSocket>>) {
        self.ports = self
            .live_pids()
            .into_iter()
            .filter_map(|(pane, pid)| sample.get(&pid).map(|sockets| (pane, sockets.clone())))
            .collect();
    }

    /// The pid of every live process, to sample their ports.
    pub fn live_root_pids(&self) -> Vec<u32> {
        self.live_pids().into_iter().map(|(_, pid)| pid).collect()
    }

    /// Where `pane`'s output stands and where its current run began.
    pub(super) fn output_position(&self, pane: PaneId) -> (OutputCursor, Option<OutputCursor>) {
        self.taps
            .get(&pane)
            .map_or((OutputCursor::new(0), None), |tap| {
                (tap.cursor(), tap.run_start())
            })
    }

    /// Feeds one chunk of `pane`'s output to its tap and tests what it
    /// completed against the pane's readiness pattern and log waits.
    pub(super) fn tap_output(&mut self, pane: PaneId, at: Option<OutputCursor>, bytes: &[u8]) {
        let lines = self.tap(pane).feed(at, bytes);
        self.test_lines(pane, &lines);
    }

    /// Tests the screen a freshly spawned run on `pane` showed when its
    /// holder was first reached. A holder streams only what its child writes
    /// after that, so the lines printed before arrive as this screen; each
    /// is taken to begin at the run's start.
    pub(super) fn observe_first_screen(&mut self, pane: PaneId) {
        let Some(text) = self
            .panes
            .get(&pane)
            .map(|target| target.parser.screen_text())
        else {
            return;
        };
        let at = self
            .output_position(pane)
            .1
            .unwrap_or_else(|| OutputCursor::new(0));
        let lines: Vec<TapLine> = text
            .lines()
            .map(str::trim_end)
            .filter(|line| !line.is_empty())
            .map(|line| TapLine::builder().at(at).text(line.to_string()).build())
            .collect();
        self.test_lines(pane, &lines);
    }

    /// Tests `lines` of `pane`'s current run against its readiness pattern
    /// and the log waits that count that run.
    fn test_lines(&mut self, pane: PaneId, lines: &[TapLine]) {
        if lines.is_empty() {
            return;
        }
        let generation = self.current_generation(pane);
        if self.readiness.get(&pane).is_some_and(|readiness| {
            readiness.state.is_waiting()
                && readiness
                    .matcher
                    .as_ref()
                    .is_some_and(|matcher| lines.iter().any(|line| matcher.is_match(line.text())))
        }) {
            self.pass_readiness(pane);
        }
        let mut index = 0;
        while index < self.waiters.len() {
            let waiter = &self.waiters[index];
            let matched = (waiter.pane() == pane && waiter.pin().counts(generation))
                .then(|| first_match(waiter, lines))
                .flatten();
            match matched {
                Some(line) => {
                    let waiter = self.waiters.remove(index);
                    self.answer(waiter, WaitStatus::Matched, Some(line));
                },
                None => index += 1,
            }
        }
    }

    /// Starts a new run on `pane`: its output continues the same stream from
    /// a new run start, and its readiness check, when it has one, waits from
    /// now.
    pub(super) fn begin_run(&mut self, pane: PaneId) {
        self.tap(pane).begin_run();
        self.readiness.remove(&pane);
        let Some(check) = self
            .workspace
            .process(pane)
            .and_then(|process| process.ready().clone())
        else {
            return;
        };
        let now = Instant::now();
        let deadline = now + check.timeout().into_inner();
        self.start_readiness(pane, check, now, deadline);
    }

    /// Takes over the run of a holder adopted from an earlier service. Its
    /// readiness check waits out what is left of its timeout; once that
    /// passed, a log check counts as passed, since the earlier service saw the
    /// lines and they are not seen again, while a port or HTTP check is
    /// probed afresh.
    pub(super) fn adopt_run(&mut self, pane: PaneId) {
        self.tap(pane).begin_run();
        self.readiness.remove(&pane);
        let Some(check) = self
            .workspace
            .process(pane)
            .and_then(|process| process.ready().clone())
        else {
            return;
        };
        let now = Instant::now();
        let since = self
            .panes
            .get(&pane)
            .map_or(now, |target| target.started_at);
        let timeout = check.timeout().into_inner();
        let deadline = since + timeout;
        match (deadline > now, check.probe()) {
            (true, _) => self.start_readiness(pane, check, since, deadline),
            (false, ReadyProbe::Log(_)) => {
                self.readiness.insert(pane, RunReadiness {
                    state: Readiness::Ready,
                    check,
                    since,
                    deadline,
                    matcher: None,
                });
            },
            (false, _) => self.start_readiness(pane, check, since, now + timeout),
        }
    }

    /// Asks the probe worker to try `pane`'s readiness probe and the probes of
    /// its waits that count the current run, replacing any it tried before.
    /// Runs when a run is installed, so a probe always names the generation
    /// whose pass the loop accepts.
    pub(super) fn arm_probes(&mut self, pane: PaneId) {
        let Some(probes) = self.probes.clone() else {
            return;
        };
        let generation = self.current_generation(pane);
        probes.forget(self.root(), pane);
        if let Some(readiness) = self
            .readiness
            .get(&pane)
            .filter(|readiness| readiness.state.is_waiting())
            && let Some(probe) = readiness.probe()
        {
            probes.watch(self.root(), pane, generation, probe, readiness.deadline);
        }
        for waiter in &self.waiters {
            if waiter.pane() == pane
                && waiter.pin().counts(generation)
                && let Some(probe) = probe_of(waiter.until())
            {
                probes.watch(self.root(), pane, generation, probe, waiter.deadline());
            }
        }
    }

    /// Ends `pane`'s run `generation`, which went `reason`: records why for
    /// `ps`, clears its readiness, ports and probes, records the event with
    /// `detail`, and answers every wait on that run, matched when it waited
    /// for `next`, the state the pane is left in.
    pub(super) fn end_run(
        &mut self,
        pane: PaneId,
        generation: SpawnGeneration,
        reason: ExitReason,
        next: ProcessState,
        detail: &str,
    ) {
        self.run_ends.insert(pane, RunEnd {
            generation,
            at: SystemTime::now(),
            reason,
        });
        if let Some(readiness) = self.readiness.get_mut(&pane) {
            readiness.state = readiness.state.on_exit();
            readiness.matcher = None;
        }
        self.ports.remove(&pane);
        if let Some(probes) = &self.probes {
            probes.forget(self.root(), pane);
        }
        let kind = match reason {
            ExitReason::Crashed | ExitReason::Killed | ExitReason::FailedToStart => {
                EventKind::Crashed
            },
            _ if next == ProcessState::Restarting => EventKind::Restarting,
            ExitReason::Stopped => EventKind::Stopped,
            ExitReason::Exited => EventKind::Exited,
        };
        self.record_event(Some(pane), kind, detail.to_string());
        self.end_waits(pane, Some(generation), reason.wait_status(), next);
    }

    /// Answers the waits on `pane` that count `generation`, or all of them
    /// when none is given: matched when one waited for the state `next`,
    /// otherwise with `status`.
    pub(super) fn end_waits(
        &mut self,
        pane: PaneId,
        generation: Option<SpawnGeneration>,
        status: WaitStatus,
        next: ProcessState,
    ) {
        let mut index = 0;
        while index < self.waiters.len() {
            let waiter = &self.waiters[index];
            if waiter.pane() == pane
                && generation.is_none_or(|generation| waiter.pin().counts(generation))
            {
                let waiter = self.waiters.remove(index);
                let status = match waiter.until() {
                    WaitCondition::State(goal) if goal.reached(next) => WaitStatus::Matched,
                    _ => status,
                };
                self.answer(waiter, status, None);
            } else {
                index += 1;
            }
        }
    }

    /// Records a lifecycle event about `pane`'s process, or the project.
    pub(super) fn record_event(&self, pane: Option<PaneId>, kind: EventKind, detail: String) {
        let process = pane
            .and_then(|pane| self.workspace.process(pane))
            .map(|process| process.name().clone());
        let cursor = pane
            .and_then(|pane| self.taps.get(&pane))
            .map(OutputTap::cursor);
        self.event_log
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .record(
                EventDraft::builder()
                    .project(Some(self.root().clone()))
                    .process(process)
                    .kind(kind)
                    .detail(detail)
                    .cursor(cursor)
                    .build(),
            );
    }

    /// `pane`'s current spawn generation.
    pub(super) fn current_generation(&self, pane: PaneId) -> SpawnGeneration {
        self.generations
            .get(&pane)
            .copied()
            .unwrap_or_else(SpawnGeneration::initial)
    }

    /// Registers `prepared` with its answer going to `reply`, answering it at
    /// once when that is already known.
    fn register(
        &mut self,
        prepared: PreparedWait,
        after: Option<SpawnGeneration>,
        reply: WaitReply,
    ) {
        let PreparedWait {
            pane,
            process,
            until,
            matcher,
            since,
            timeout,
        } = prepared;
        let current = self.current_generation(pane);
        let pin = after.map_or(RunPin::From(current), RunPin::After);
        let now = Instant::now();
        let mut matched = None;
        let mut scanned_from = None;
        if let (Some(since), Some(matcher), false) = (since, matcher.as_ref(), pin.is_fresh()) {
            let (lines, from) = self.tap(pane).scan_from(since);
            scanned_from = from;
            matched = lines
                .into_iter()
                .find(|line| matcher.is_match(line.text()))
                .map(matched_line);
        }
        let waiter = Waiter::builder()
            .pane(pane)
            .process(process)
            .until(until)
            .matcher(matcher)
            .pin(pin)
            .started(now)
            .deadline(now + timeout)
            .reply(reply)
            .scanned_from(scanned_from)
            .build();
        let settled = match matched {
            Some(line) => Some((WaitStatus::Matched, Some(line))),
            None => self
                .check(&waiter)
                .or_else(|| self.not_running(&waiter).map(|status| (status, None))),
        };
        if let Some((status, matched)) = settled {
            self.answer(waiter, status, matched);
            return;
        }
        if let (Some(probes), Some(probe), true) = (
            self.probes.as_ref(),
            probe_of(waiter.until()),
            self.is_live(pane) && pin.counts(current),
        ) {
            probes.watch(self.root(), pane, current, probe, waiter.deadline());
        }
        self.waiters.push(waiter);
    }

    /// How `waiter` ends now when its state or readiness condition holds, or
    /// its readiness check timed out.
    fn check(&self, waiter: &Waiter) -> Option<(WaitStatus, Option<MatchedLine>)> {
        let pane = waiter.pane();
        if !waiter.pin().counts(self.current_generation(pane)) {
            return None;
        }
        match waiter.until() {
            WaitCondition::State(goal) => self
                .workspace
                .process(pane)
                .filter(|process| goal.reached(*process.state()))
                .map(|_| (WaitStatus::Matched, None)),
            WaitCondition::Ready => {
                match self.readiness.get(&pane).map(|readiness| readiness.state) {
                    Some(Readiness::Ready) => Some((WaitStatus::Matched, None)),
                    Some(Readiness::TimedOut) => Some((WaitStatus::ReadyTimeout, None)),
                    _ => None,
                }
            },
            _ => None,
        }
    }

    /// How `waiter` ends at once because there is no run for it to wait on:
    /// not running when nothing runs, or as its run already ended when a start
    /// it rides on ended before it registered.
    fn not_running(&self, waiter: &Waiter) -> Option<WaitStatus> {
        let pane = waiter.pane();
        if self.is_live(pane) || self.is_starting(pane) {
            return None;
        }
        if let Some(end) = self
            .run_ends
            .get(&pane)
            .filter(|end| waiter.pin().is_fresh() && waiter.pin().counts(end.generation))
        {
            return Some(end.reason.wait_status());
        }
        let state = self.workspace.process(pane).map(|process| *process.state());
        match waiter.pin() {
            RunPin::After(after) if self.current_generation(pane) == after => {
                Some(WaitStatus::NotRunning)
            },
            RunPin::From(_) if state.is_none_or(|state| !state.may_be_running()) => {
                Some(WaitStatus::NotRunning)
            },
            _ => None,
        }
    }

    /// Answers `waiter` with `status`, the line that matched, where the
    /// output stands, and the run's last lines when it did not match.
    fn answer(&mut self, waiter: Waiter, status: WaitStatus, matched: Option<MatchedLine>) {
        let pane = waiter.pane();
        let (cursor, _) = self.output_position(pane);
        let tail = if status == WaitStatus::Matched {
            Vec::new()
        } else {
            self.taps
                .get(&pane)
                .map(|tap| tap.tail(FAILURE_TAIL_LINES))
                .unwrap_or_default()
        };
        let outcome = WaitOutcome::builder()
            .process(waiter.process().clone())
            .status(status)
            .until(waiter.until().clone())
            .matched(matched)
            .cursor(cursor)
            .elapsed(waiter.started().elapsed())
            .tail(tail)
            .scanned_from(waiter.scanned_from())
            .build();
        match waiter.into_reply() {
            WaitReply::Direct(reply) => {
                let _ = reply.send(ServiceResponse::Waited(outcome));
            },
            WaitReply::Batch { group, slot } => {
                if self
                    .wait_groups
                    .get_mut(&group)
                    .is_some_and(|waits| waits.fill(slot, outcome))
                {
                    self.wait_groups.remove(&group);
                }
            },
        }
    }

    /// Starts `pane`'s readiness under `check`, from `since` until
    /// `deadline`; a log pattern that does not compile is told once and the
    /// run has no readiness.
    fn start_readiness(
        &mut self,
        pane: PaneId,
        check: ReadyCheck,
        since: Instant,
        deadline: Instant,
    ) {
        let matcher = match check.probe() {
            ReadyProbe::Log(pattern) => match LineMatcher::compile(pattern) {
                Ok(matcher) => Some(matcher),
                Err(error) => {
                    self.notify(
                        NoticeLevel::Warning,
                        Some(pane),
                        format!("{READY_PATTERN_NOTICE}: {error}"),
                    );
                    return;
                },
            },
            _ => None,
        };
        self.readiness.insert(pane, RunReadiness {
            state: Readiness::on_spawn(Some(&check)),
            check,
            since,
            deadline,
            matcher,
        });
    }

    /// Passes `pane`'s readiness check and records the event.
    fn pass_readiness(&mut self, pane: PaneId) {
        let Some(readiness) = self.readiness.get_mut(&pane) else {
            return;
        };
        readiness.state = readiness.state.on_passed();
        readiness.matcher = None;
        let took = humantime::format_duration(whole_millis(readiness.since.elapsed()));
        self.record_event(Some(pane), EventKind::Ready, format!("ready in {took}"));
    }

    /// Times out `pane`'s readiness check: a warning notice and an event; the
    /// process keeps running.
    fn time_out_readiness(&mut self, pane: PaneId) {
        let Some(readiness) = self.readiness.get_mut(&pane) else {
            return;
        };
        readiness.state = readiness.state.on_deadline();
        readiness.matcher = None;
        let timeout = *readiness.check.timeout();
        let name = self
            .workspace
            .process(pane)
            .map(|process| process.name().to_string())
            .unwrap_or_default();
        let message = format!("{name} did not become ready within {timeout}");
        self.notify(NoticeLevel::Warning, Some(pane), message.clone());
        self.record_event(Some(pane), EventKind::ReadyTimeout, message);
    }

    /// `pane`'s output tap, made on first use.
    fn tap(&mut self, pane: PaneId) -> &mut OutputTap {
        self.taps
            .entry(pane)
            .or_insert_with(|| OutputTap::new(OutputCursor::new(0)))
    }
}

/// The probe a port or HTTP condition is tested with.
fn probe_of(condition: &WaitCondition) -> Option<ProbeTarget> {
    match condition {
        WaitCondition::Port(port) => Some(ProbeTarget::Port(*port)),
        WaitCondition::Http { url, status } => Some(ProbeTarget::Http {
            url: url.clone(),
            status: *status,
        }),
        _ => None,
    }
}

/// The first of `lines` that `waiter`'s log pattern matches.
fn first_match(waiter: &Waiter, lines: &[TapLine]) -> Option<MatchedLine> {
    let matcher = waiter.matcher().as_ref()?;
    lines
        .iter()
        .find(|line| matcher.is_match(line.text()))
        .cloned()
        .map(matched_line)
}

/// `line` as the line a wait matched.
fn matched_line(line: TapLine) -> MatchedLine {
    MatchedLine::builder()
        .at(line.at())
        .line(line.text().clone())
        .build()
}

/// `duration` to the millisecond, so it reads as `2s 140ms`.
fn whole_millis(duration: Duration) -> Duration {
    Duration::from_millis(u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
}

/// A refusal of a value in the request, saying `message`.
fn invalid_input(message: String) -> ServiceErrorBody {
    ServiceErrorBody::builder()
        .kind(ServiceErrorKind::InvalidInput)
        .message(message)
        .build()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use crossbeam_channel::{Receiver, unbounded};

    use super::*;
    use crate::{
        adapter::service::{LaunchEnvironment, StackEvent, StateGoal, supervisor::testing::*},
        domain::{
            config::ProcessSpec,
            process::{LogPattern, PatternKind, PatternText, RestartPolicy},
            pty::{ExitCause, ExitOutcome, ProcessOutput},
            value::{LocalPort, PositiveDuration},
        },
    };

    /// How long every test wait may take.
    const WAIT: Duration = Duration::from_secs(30);

    /// A substring pattern.
    fn log(text: &str) -> WaitCondition {
        WaitCondition::Log(
            LogPattern::builder()
                .text(PatternText::try_new(text).unwrap())
                .kind(PatternKind::Substring)
                .build(),
        )
    }

    /// A wait for `until` from `since`.
    fn spec(until: Option<WaitCondition>, since: Option<OutputCursor>) -> WaitSpec {
        WaitSpec::builder()
            .until(until)
            .since(since)
            .timeout(WAIT)
            .build()
    }

    /// A readiness check of `probe` with a 30 s timeout.
    fn ready(probe: ReadyProbe) -> ReadyCheck {
        ReadyCheck::builder()
            .probe(probe)
            .timeout(PositiveDuration::try_new(WAIT).unwrap())
            .build()
    }

    /// The process `api` under `restart`, with `check`.
    fn api(restart: RestartPolicy, check: Option<ReadyCheck>) -> ProcessSpec {
        super::super::testing::spec("api", "true", restart).with_ready(check)
    }

    /// A harness running `processes`, every one brought up.
    fn up(processes: &[ProcessSpec]) -> Harness {
        let mut harness = Harness::over(Box::new(FakeRunner), processes);
        harness.up();
        harness
    }

    /// Feeds `bytes` as `name`'s current run's output.
    fn print(harness: &mut Harness, name: &str, bytes: &[u8]) {
        let pane = harness
            .supervisor
            .pane_named(&super::super::testing::name(name))
            .unwrap();
        let generation = harness.supervisor.current_generation(pane);
        harness
            .supervisor
            .handle_output(pane, generation, ProcessOutput::Chunk(bytes.to_vec()));
    }

    /// Ends `name`'s current run with `outcome`.
    fn end(harness: &mut Harness, name: &str, outcome: ExitOutcome) {
        let pane = harness
            .supervisor
            .pane_named(&super::super::testing::name(name))
            .unwrap();
        let generation = harness.supervisor.current_generation(pane);
        harness
            .supervisor
            .handle_output(pane, generation, ProcessOutput::Exited(outcome));
    }

    /// Registers a wait on `name`, pinned past `after` when given.
    fn wait_on(
        harness: &mut Harness,
        name: &str,
        wait: &WaitSpec,
        after: Option<SpawnGeneration>,
    ) -> Receiver<ServiceResponse> {
        let (reply, answers) = unbounded();
        let prepared = harness
            .supervisor
            .prepare_wait(&super::super::testing::name(name), wait)
            .unwrap();
        harness.supervisor.begin_wait(prepared, after, reply);
        answers
    }

    /// The wait outcome answered on `answers`, if any yet.
    fn answered(answers: &Receiver<ServiceResponse>) -> Option<WaitOutcome> {
        match answers.try_recv() {
            Ok(ServiceResponse::Waited(outcome)) => Some(outcome),
            Ok(_) => panic!("a wait answers with how it ended"),
            Err(_) => None,
        }
    }

    /// The kinds of the events recorded so far.
    fn event_kinds(harness: &Harness) -> Vec<EventKind> {
        harness
            .event_log
            .lock()
            .unwrap()
            .page(harness.supervisor.root(), None, &[])
            .events()
            .iter()
            .map(StackEvent::kind)
            .collect()
    }

    #[test]
    fn a_log_wait_matches_only_output_after_it_unless_given_a_cursor() {
        let mut harness = up(&[api(RestartPolicy::Never, None)]);
        print(&mut harness, "api", b"Listening on :3000\n");

        let fresh = wait_on(
            &mut harness,
            "api",
            &spec(Some(log("Listening")), None),
            None,
        );
        let resumed = wait_on(
            &mut harness,
            "api",
            &spec(Some(log("Listening")), Some(OutputCursor::new(0))),
            None,
        );

        assert!(answered(&fresh).is_none(), "earlier output does not count");
        let matched = answered(&resumed).unwrap();
        assert_eq!(matched.status(), WaitStatus::Matched);
        assert_eq!(
            matched.matched().as_ref().unwrap().line(),
            "Listening on :3000"
        );
        assert_eq!(*matched.cursor(), OutputCursor::new(19));

        print(&mut harness, "api", b"\x1b[32mListening\x1b[0m again\n");
        let again = answered(&fresh).unwrap();
        assert_eq!(again.matched().as_ref().unwrap().line(), "Listening again");
        assert_eq!(
            *again.matched().as_ref().unwrap().at(),
            OutputCursor::new(19)
        );
    }

    #[test]
    fn a_wait_riding_on_a_restart_never_sees_the_old_run() {
        let mut harness = up(&[api(RestartPolicy::Never, None)]);
        let name = super::super::testing::name("api");
        let before = harness.supervisor.generation_of(&name);
        harness
            .supervisor
            .restart(&name, &LaunchEnvironment::default());
        let answers = wait_on(
            &mut harness,
            "api",
            &spec(Some(log("ready")), None),
            Some(before),
        );

        print(&mut harness, "api", b"ready (old run, shutting down)\n");
        end(
            &mut harness,
            "api",
            ExitOutcome::Failed(ExitCause::Status(143)),
        );
        assert!(
            answered(&answers).is_none(),
            "the old run's lines and exit do not count"
        );

        print(&mut harness, "api", b"ready\n");
        assert_eq!(answered(&answers).unwrap().status(), WaitStatus::Matched);
    }

    #[test]
    fn a_crash_ends_a_wait_with_the_run_s_last_lines() {
        let mut harness = up(&[api(RestartPolicy::Never, None)]);
        let answers = wait_on(
            &mut harness,
            "api",
            &spec(Some(log("Listening")), None),
            None,
        );

        print(&mut harness, "api", b"Error: EADDRINUSE\n    at listen\n");
        end(
            &mut harness,
            "api",
            ExitOutcome::Failed(ExitCause::Status(1)),
        );

        let crashed = answered(&answers).unwrap();
        assert_eq!(crashed.status(), WaitStatus::Crashed);
        assert_eq!(crashed.tail(), &["Error: EADDRINUSE", "    at listen"]);
        assert!(event_kinds(&harness).contains(&EventKind::Crashed));
    }

    #[test]
    fn a_process_that_is_not_running_answers_at_once() {
        let mut harness = Harness::over(Box::new(FakeRunner), &[api(RestartPolicy::Never, None)]);

        let answers = wait_on(&mut harness, "api", &spec(Some(log("x")), None), None);
        let stopped = wait_on(
            &mut harness,
            "api",
            &spec(Some(WaitCondition::State(StateGoal::Stopped)), None),
            None,
        );

        assert_eq!(answered(&answers).unwrap().status(), WaitStatus::NotRunning);
        assert_eq!(answered(&stopped).unwrap().status(), WaitStatus::Matched);
    }

    #[test]
    fn a_wait_past_its_deadline_answers_waiting_with_its_cursor() {
        let mut harness = up(&[api(RestartPolicy::Never, None)]);
        print(&mut harness, "api", b"booting\n");
        let answers = wait_on(&mut harness, "api", &spec(Some(log("never")), None), None);

        harness.supervisor.settle_waits(Instant::now());
        assert!(answered(&answers).is_none());
        harness.supervisor.settle_waits(Instant::now() + WAIT);

        let waiting = answered(&answers).unwrap();
        assert_eq!(waiting.status(), WaitStatus::Waiting);
        assert_eq!(*waiting.cursor(), OutputCursor::new(8));
        assert!(harness.supervisor.next_wait_deadline().is_none());
    }

    #[test]
    fn a_log_readiness_check_passes_on_its_line_and_answers_a_ready_wait() {
        let check = ready(ReadyProbe::from_stored("log", "compiled").unwrap());
        let mut harness = up(&[api(RestartPolicy::Never, Some(check))]);
        let answers = wait_on(&mut harness, "api", &spec(None, None), None);
        assert_eq!(
            *harness.supervisor.snapshot().processes()[0].ready(),
            Readiness::Waiting
        );

        print(&mut harness, "api", b"webpack compiled in 3ms\n");
        harness.supervisor.settle_waits(Instant::now());

        let ready = answered(&answers).unwrap();
        assert_eq!(ready.status(), WaitStatus::Matched);
        assert_eq!(*ready.until(), WaitCondition::Ready);
        assert_eq!(
            *harness.supervisor.snapshot().processes()[0].ready(),
            Readiness::Ready
        );
        assert!(event_kinds(&harness).contains(&EventKind::Ready));
    }

    #[test]
    fn a_readiness_check_that_times_out_warns_and_fails_its_wait() {
        let check = ready(ReadyProbe::from_stored("log", "compiled").unwrap());
        let mut harness = up(&[api(RestartPolicy::Never, Some(check))]);
        let answers = wait_on(&mut harness, "api", &spec(None, None), None);

        harness.supervisor.settle_waits(Instant::now() + WAIT);

        assert_eq!(
            answered(&answers).unwrap().status(),
            WaitStatus::ReadyTimeout
        );
        assert_eq!(
            *harness.supervisor.snapshot().processes()[0].ready(),
            Readiness::TimedOut
        );
        assert_eq!(harness.notice_messages(), [
            "api did not become ready within 30s"
        ]);
        assert!(event_kinds(&harness).contains(&EventKind::ReadyTimeout));
        assert_eq!(
            *harness.supervisor.snapshot().processes()[0].state(),
            ProcessState::Running
        );
    }

    #[test]
    fn a_probe_pass_counts_only_for_the_current_run() {
        let port = LocalPort::try_new(3000).unwrap();
        let mut harness = up(&[api(
            RestartPolicy::Never,
            Some(ready(ReadyProbe::Port(port))),
        )]);
        let pane = harness
            .supervisor
            .pane_named(&super::super::testing::name("api"))
            .unwrap();
        let current = harness.supervisor.current_generation(pane);
        let stale = SpawnGeneration::initial();

        harness
            .supervisor
            .handle_probe_passed(pane, stale, &ProbeTarget::Port(port));
        assert_eq!(
            *harness.supervisor.snapshot().processes()[0].ready(),
            Readiness::Waiting
        );

        harness
            .supervisor
            .handle_probe_passed(pane, current, &ProbeTarget::Port(port));
        assert_eq!(
            *harness.supervisor.snapshot().processes()[0].ready(),
            Readiness::Ready
        );
    }

    #[test]
    fn a_stop_while_waiting_answers_stopped_or_matched_for_a_stopped_goal() {
        let mut harness = up(&[api(RestartPolicy::Always, None)]);
        let for_log = wait_on(&mut harness, "api", &spec(Some(log("x")), None), None);
        let for_stop = wait_on(
            &mut harness,
            "api",
            &spec(Some(WaitCondition::State(StateGoal::Stopped)), None),
            None,
        );

        harness.supervisor.stop(&super::super::testing::name("api"));
        end(
            &mut harness,
            "api",
            ExitOutcome::Failed(ExitCause::Status(143)),
        );

        assert_eq!(answered(&for_log).unwrap().status(), WaitStatus::Stopped);
        assert_eq!(answered(&for_stop).unwrap().status(), WaitStatus::Matched);
        let last = harness.supervisor.snapshot().processes()[0]
            .last_exit()
            .clone()
            .unwrap();
        assert_eq!(last.reason(), ExitReason::Stopped);
    }

    #[test]
    fn an_up_with_a_wait_answers_one_batch_once_every_wait_ended() {
        let mut harness = Harness::over(Box::new(FakeRunner), &[
            api(RestartPolicy::Never, None),
            super::super::testing::spec("web", "true", RestartPolicy::Never),
        ]);
        let (reply, answers) = unbounded();

        harness
            .supervisor
            .up_waiting(
                &LaunchEnvironment::default(),
                &spec(Some(log("up")), None),
                reply,
            )
            .unwrap();
        print(&mut harness, "api", b"up\n");
        assert!(answers.try_recv().is_err(), "web still waits");
        end(
            &mut harness,
            "web",
            ExitOutcome::Failed(ExitCause::Status(2)),
        );

        match answers.try_recv() {
            Ok(ServiceResponse::Batch(batch)) => {
                let statuses: Vec<WaitStatus> =
                    batch.waits().iter().map(WaitOutcome::status).collect();
                assert_eq!(statuses, [WaitStatus::Matched, WaitStatus::Crashed]);
                assert_eq!(batch.acted().len(), 2);
            },
            _ => panic!("the batch answers once both waits ended"),
        }
    }

    #[test]
    fn a_bad_pattern_or_a_readiness_wait_without_a_check_is_refused() {
        let harness = up(&[api(RestartPolicy::Never, None)]);
        let regex = WaitCondition::Log(
            LogPattern::builder()
                .text(PatternText::try_new("(").unwrap())
                .kind(PatternKind::Regex)
                .build(),
        );
        let name = super::super::testing::name("api");

        for until in [regex, WaitCondition::Ready] {
            let refusal = harness
                .supervisor
                .prepare_wait(&name, &spec(Some(until), None))
                .err()
                .unwrap();
            assert_eq!(*refusal.kind(), ServiceErrorKind::InvalidInput);
        }
    }

    #[test]
    fn a_restart_that_fails_to_launch_ends_its_wait_as_crashed() {
        let mut harness = Harness::over(
            Box::new(FlakyRunner {
                spawns: AtomicUsize::new(0),
            }),
            &[api(RestartPolicy::Never, None)],
        );
        harness.up();
        let name = super::super::testing::name("api");
        let before = harness.supervisor.generation_of(&name);
        harness
            .supervisor
            .restart(&name, &LaunchEnvironment::default());
        let answers = wait_on(&mut harness, "api", &spec(None, None), Some(before));

        end(
            &mut harness,
            "api",
            ExitOutcome::Failed(ExitCause::Status(143)),
        );

        assert_eq!(answered(&answers).unwrap().status(), WaitStatus::Crashed);
        let last = harness.supervisor.snapshot().processes()[0]
            .last_exit()
            .clone()
            .unwrap();
        assert_eq!(last.reason(), ExitReason::FailedToStart);
        assert_eq!(*last.outcome(), ExitOutcome::FAILED_UNKNOWN);
    }

    #[test]
    fn a_start_and_an_exit_are_recorded_as_events() {
        let mut harness = up(&[api(RestartPolicy::Never, None)]);

        end(&mut harness, "api", ExitOutcome::Succeeded);

        assert_eq!(event_kinds(&harness), [
            EventKind::Started,
            EventKind::Exited
        ]);
    }
}
