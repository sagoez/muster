use std::{
    collections::{BTreeMap, HashSet},
    ffi::OsString,
    path::Path,
    thread,
    time::{Duration, Instant, SystemTime},
};

use super::{
    channel_output_sink::ChannelOutputSink, config_membership::ConfigMembership,
    entity::Supervisor, launch_purpose::LaunchPurpose, pane::Pane, pending_intent::PendingIntent,
    pending_launch::PendingLaunch, shutdown_generation::ShutdownGeneration,
    spawn_generation::SpawnGeneration, stale_release::StaleRelease,
};
use crate::{
    adapter::{
        holder::abandon,
        service::{
            BatchOutcome, EventKind, ExitReason, LaunchEnvironment, NoticeLevel, Refusal,
            ServiceEvent, WaitStatus,
        },
        terminal::{KittyKeyboardTracker, TerminalEmulator},
    },
    application::{ExitDecision, ProcessLifecycle},
    domain::{
        notification::Notification,
        port::ProcessHandle,
        process::{ExitIntent, ProcessState, StopPolicy},
        pty::{ExitOutcome, ProcessOutput, PtyError, SpawnHistory, SpawnRequest},
        value::{PaneId, ProcessName},
    },
};

/// Delay before the first automatic restart of a failing process.
const RESTART_BACKOFF_BASE: Duration = Duration::from_millis(200);
/// Upper bound on the automatic-restart delay.
const RESTART_BACKOFF_MAX: Duration = Duration::from_secs(5);
/// Cap on the backoff exponent (delay = base * 2^exp, clamped to the max).
const RESTART_BACKOFF_MAX_EXP: u32 = 5;
/// A process that ran at least this long counts as stable; its next restart
/// resets the backoff so a healthy long-lived process is not penalized.
const RESTART_STABLE_RUN: Duration = Duration::from_secs(10);
/// Shown when a graceful shutdown deadline expires and the process group is
/// force-killed.
pub(super) const FORCE_STOP_NOTICE: &str = "graceful shutdown timed out; force-killed";
/// Shown when graceful signal delivery fails and an immediate kill is used.
const GRACEFUL_STOP_FALLBACK_NOTICE: &str = "graceful shutdown failed; force-killed";
/// Shown when neither the requested signal nor a hard kill can be delivered.
pub(super) const STOP_DELIVERY_FAILED_NOTICE: &str = "could not stop the process";
/// Shown when a pause signal cannot be delivered.
const PAUSE_FAILED_NOTICE: &str = "could not pause the process";
/// Shown when a resume signal cannot be delivered.
const RESUME_FAILED_NOTICE: &str = "could not resume the process";
/// Prefix for a process that could not be spawned.
const SPAWN_FAILED_NOTICE: &str = "could not start the process";
/// Shown when a start targets a pane the workspace holds no process for.
const SPAWN_UNKNOWN_PROCESS: &str = "could not start: the pane has no process";
/// Prefix for a process whose output stream broke; the process may still run.
const OUTPUT_UNREADABLE_NOTICE: &str =
    "lost the connection to this process; it may still be running";
/// Start of the notice for an exit nobody asked for that failed, before how
/// it ended.
const CRASHED_NOTICE: &str = "crashed";
/// Shown when the service's copy of a process's screen failed on its output
/// and was reset.
const SCREEN_RESET_NOTICE: &str =
    "the terminal emulator failed on this process's output; its screen was reset";
/// Shown when a handled exit cannot be acknowledged, so its holder keeps it.
const EXIT_ACKNOWLEDGE_FAILED_NOTICE: &str =
    "could not release the finished process; a later start may report its exit again";

impl Supervisor {
    /// Starts every autostart process that is not already running, each with
    /// `environment`, the environment of the command that brought the project
    /// up. Returns what it started and what it left stopped, with why.
    pub fn up(&mut self, environment: &LaunchEnvironment) -> BatchOutcome {
        let before: HashSet<PaneId> = self
            .workspace
            .processes()
            .iter()
            .map(|process| *process.id())
            .filter(|pane| self.is_live(*pane) || self.is_starting(*pane))
            .collect();
        self.batch_refusals.replace(Some(Vec::new()));
        for pane in self.autostart_panes() {
            if self.refuse_blocked_launch(pane) {
                continue;
            }
            // Bringing a process up, like starting it, begins a new streak.
            self.crash_streaks.remove(&pane);
            self.set_launch_environment(pane, environment);
            self.spawn(pane);
        }
        let acted: Vec<ProcessName> = self
            .workspace
            .processes()
            .iter()
            .filter(|process| {
                let pane = *process.id();
                !before.contains(&pane) && (self.is_live(pane) || self.is_starting(pane))
            })
            .map(|process| process.name().clone())
            .collect();
        self.finish_batch(acted, Vec::new())
    }

    /// Ends a project-wide request that acted on `acted`: every error it
    /// raised about a process it did not act on, then `left_stopped`, are the
    /// refusals.
    fn finish_batch(&self, acted: Vec<ProcessName>, left_stopped: Vec<Refusal>) -> BatchOutcome {
        let mut refused: Vec<Refusal> = self
            .batch_refusals
            .take()
            .unwrap_or_default()
            .into_iter()
            .filter(|refusal| !acted.contains(refusal.name()))
            .collect();
        refused.extend(left_stopped);
        BatchOutcome::builder()
            .acted(acted)
            .refused(refused)
            .build()
    }

    /// Stops every running process through its stop policy. Pending restarts
    /// are cancelled. Returns what it stopped and what it could not, with why.
    pub fn down(&mut self) -> BatchOutcome {
        let processes: Vec<(PaneId, ProcessName)> = self
            .workspace
            .processes()
            .iter()
            .filter(|process| process.state().may_be_running())
            .map(|process| (*process.id(), process.name().clone()))
            .collect();
        self.batch_refusals.replace(Some(Vec::new()));
        for (pane, _) in &processes {
            self.stop_pane(*pane);
        }
        let refused: Vec<ProcessName> = self
            .batch_refusals
            .borrow()
            .iter()
            .flatten()
            .map(|refusal| refusal.name().clone())
            .collect();
        let acted = processes
            .into_iter()
            .map(|(_, name)| name)
            .filter(|name| !refused.contains(name))
            .collect();
        self.finish_batch(acted, Vec::new())
    }

    /// Starts the process named `name` unless it is already running, with
    /// `environment`, the environment of the command that asked.
    pub fn start(&mut self, name: &ProcessName, environment: &LaunchEnvironment) {
        if let Some(pane) = self.resolve(name) {
            self.start_pane(pane, environment);
        }
    }

    /// Stops the process named `name` without letting its restart policy
    /// respawn it.
    pub fn stop(&mut self, name: &ProcessName) {
        if let Some(pane) = self.resolve(name) {
            self.stop_pane(pane);
        }
    }

    /// Restarts the process named `name` regardless of its restart policy,
    /// with `environment`, the environment of the command that asked.
    pub fn restart(&mut self, name: &ProcessName, environment: &LaunchEnvironment) {
        if let Some(pane) = self.resolve(name) {
            self.set_launch_environment(pane, environment);
            self.restart_pane(pane);
        }
    }

    /// Suspends the running process named `name`.
    pub fn pause(&mut self, name: &ProcessName) {
        if let Some(pane) = self.resolve(name) {
            self.pause_pane(pane);
        }
    }

    /// Resumes the paused process named `name`.
    pub fn resume(&mut self, name: &ProcessName) {
        if let Some(pane) = self.resolve(name) {
            self.resume_pane(pane);
        }
    }

    /// Force-kills the process named `name` at once, skipping its graceful
    /// stop, and cancels a pending respawn.
    pub fn force_kill(&mut self, name: &ProcessName) {
        if let Some(pane) = self.resolve(name) {
            self.force_kill_pane(pane);
        }
    }

    /// Makes `environment` the one `pane` is launched with, now and on its
    /// policy restarts.
    fn set_launch_environment(&mut self, pane: PaneId, environment: &LaunchEnvironment) {
        self.launch_environments.insert(pane, environment.to_os());
    }

    /// Starts `pane` with `environment` unless it is already running or
    /// starting.
    fn start_pane(&mut self, pane: PaneId, environment: &LaunchEnvironment) {
        if self.is_live(pane) || self.is_starting(pane) || self.refuse_blocked_launch(pane) {
            return;
        }
        // A start the user asks for begins a new streak, so its failure is told.
        self.failing_spawns.remove(&pane);
        self.crash_streaks.remove(&pane);
        self.set_launch_environment(pane, environment);
        self.spawn(pane);
    }

    /// Restarts `pane` regardless of its restart policy: a live process is
    /// stopped gracefully and respawned on its exit, a stopped one is spawned
    /// at once.
    pub(crate) fn restart_pane(&mut self, pane: PaneId) {
        if self.record_intent(pane, PendingIntent::Restart) {
            return;
        }
        self.failing_spawns.remove(&pane);
        self.crash_streaks.remove(&pane);
        if self.is_live(pane) {
            if self
                .panes
                .get(&pane)
                .is_some_and(|target| !target.exit_intent.accepts_restart_request())
            {
                return;
            }
            self.request_graceful_transition(pane, ExitIntent::request_restart, true);
            return;
        }
        if self.refuse_blocked_launch(pane) {
            return;
        }
        // A pane already in restart backoff is counted by `spawn` itself.
        let counted_by_spawn = self
            .workspace
            .process(pane)
            .is_some_and(|process| *process.state() == ProcessState::Restarting);
        let before = self.generations.get(&pane).copied();
        self.spawn(pane);
        if counted_by_spawn || self.generations.get(&pane).copied() == before {
            return;
        }
        if self.is_live(pane) {
            self.count_restart(pane);
        } else if let Some(LaunchPurpose::Spawn { restarting }) = self
            .launches
            .get_mut(&pane)
            .map(|launch| &mut launch.purpose)
        {
            *restarting = true;
        }
    }

    /// Stops `pane` without letting its restart policy respawn it.
    pub(crate) fn stop_pane(&mut self, pane: PaneId) {
        if self.record_intent(pane, PendingIntent::Stop) {
            return;
        }
        if self.is_unsupervised(pane) {
            self.stop_unsupervised(pane, false);
            return;
        }
        if self.panes.get(&pane).is_some_and(|target| {
            target.handle.is_some() && !target.exit_intent.accepts_stop_request()
        }) {
            return;
        }
        if self.is_live(pane) {
            self.request_graceful_transition(pane, ExitIntent::request_stop, false);
        } else {
            self.settle_stopped(pane);
        }
    }

    /// Settles a stop or kill of `pane` while it runs nothing: a pending
    /// restart is cancelled and every wait on it ends as stopped.
    fn settle_stopped(&mut self, pane: PaneId) {
        self.bump_generation(pane);
        self.deactivate(pane);
        self.workspace.set_state(pane, ProcessState::Exited);
        self.end_waits(pane, None, WaitStatus::Stopped, ProcessState::Exited);
    }

    /// Suspends `pane`'s live process unless it is already paused. A pane
    /// still starting is paused once its launch settles.
    pub(crate) fn pause_pane(&mut self, pane: PaneId) {
        if self.record_suspend(pane, true) {
            return;
        }
        if !self.is_paused(pane) {
            self.signal_pause(pane, true);
        }
    }

    /// Resumes `pane`'s live process when it is paused. A pane still starting
    /// has a pause asked for meanwhile cancelled, or is resumed once its
    /// launch settles.
    pub(crate) fn resume_pane(&mut self, pane: PaneId) {
        if self.record_suspend(pane, false) {
            return;
        }
        if self.is_paused(pane) {
            self.signal_pause(pane, false);
        }
    }

    /// Records a pause (`pause`) or resume for `pane` when it is starting, to
    /// apply once its launch settles. Returns whether it was recorded.
    fn record_suspend(&mut self, pane: PaneId, pause: bool) -> bool {
        match self.launches.get_mut(&pane) {
            Some(launch) => {
                launch.suspend = Some(pause);
                true
            },
            None => false,
        }
    }

    /// Immediately force-kills `pane` and cancels a pending respawn.
    pub(crate) fn force_kill_pane(&mut self, pane: PaneId) {
        if self.record_intent(pane, PendingIntent::Kill) {
            return;
        }
        if self.is_unsupervised(pane) {
            self.stop_unsupervised(pane, true);
            return;
        }
        if !self.is_live(pane) {
            self.settle_stopped(pane);
            return;
        }
        self.advance_shutdown_generation(pane);
        let mut delivered = false;
        if let Some(target) = self.panes.get_mut(&pane) {
            target.exit_intent = target.exit_intent.request_stop();
            if let Some(handle) = target.handle.as_mut() {
                delivered = handle.kill().is_ok();
                target.exit_intent = if delivered {
                    target.exit_intent.stop_delivered()
                } else {
                    target.exit_intent.stop_delivery_failed()
                };
            }
        }
        if delivered {
            self.workspace.set_state(pane, ProcessState::Stopping);
        } else {
            self.notify(NoticeLevel::Error, Some(pane), STOP_DELIVERY_FAILED_NOTICE);
        }
    }

    /// Whether `pane` has a live process handle.
    pub(crate) fn is_live(&self, pane: PaneId) -> bool {
        self.panes
            .get(&pane)
            .is_some_and(|target| target.handle.is_some())
    }

    /// Whether `pane`'s process is paused.
    fn is_paused(&self, pane: PaneId) -> bool {
        self.workspace
            .process(pane)
            .is_some_and(|process| *process.state() == ProcessState::Paused)
    }

    /// Delivers a pause (`pause`) or resume to `pane`'s live process when no stop
    /// or restart is in flight, recording the new state only once it lands.
    fn signal_pause(&mut self, pane: PaneId, pause: bool) {
        let Some(target) = self.panes.get_mut(&pane) else {
            return;
        };
        if target.exit_intent != ExitIntent::FollowPolicy {
            return;
        }
        let Some(handle) = target.handle.as_mut() else {
            return;
        };
        let signalled = if pause {
            handle.pause()
        } else {
            handle.resume()
        };
        match (signalled, pause) {
            (Ok(()), true) => self.workspace.set_state(pane, ProcessState::Paused),
            (Ok(()), false) => self.workspace.set_state(pane, ProcessState::Running),
            (Err(_), true) => self.notify(NoticeLevel::Error, Some(pane), PAUSE_FAILED_NOTICE),
            (Err(_), false) => self.notify(NoticeLevel::Error, Some(pane), RESUME_FAILED_NOTICE),
        }
    }

    /// Applies one process output event unless a newer spawn superseded it. A
    /// chunk updates the pane's terminal and answers the child's terminal
    /// queries, unless its handle answers them itself. An unreadable stream
    /// becomes a notice naming the process, which is then reconnected.
    pub fn handle_output(
        &mut self,
        pane: PaneId,
        generation: SpawnGeneration,
        output: ProcessOutput,
    ) {
        if self.generations.get(&pane) != Some(&generation) {
            return;
        }
        if let Some(launch) = self
            .launches
            .get_mut(&pane)
            .filter(|launch| launch.generation == generation)
        {
            launch.keep(output);
            return;
        }
        match output {
            ProcessOutput::Chunk(bytes) => {
                if self.paint(pane, &bytes) {
                    self.tap_output(pane, None, &bytes);
                }
            },
            ProcessOutput::Placed { at, bytes } => {
                if self.paint(pane, &bytes) {
                    self.tap_output(pane, Some(at), &bytes);
                }
            },
            ProcessOutput::Repaint(screen) => {
                self.paint(pane, &screen);
            },
            ProcessOutput::Warning(warning) => {
                self.notify(NoticeLevel::Warning, Some(pane), warning);
            },
            ProcessOutput::Attached(writable_clients) => {
                if self.is_live(pane) {
                    self.set_pane_attached(pane, writable_clients);
                }
            },
            ProcessOutput::Unreadable(reason) => {
                self.notify(
                    NoticeLevel::Error,
                    Some(pane),
                    format!("{OUTPUT_UNREADABLE_NOTICE}: {reason}"),
                );
                self.reconnect(pane);
            },
            ProcessOutput::Exited(outcome) => self.handle_exit(pane, outcome),
        }
    }

    /// Paints `bytes` onto `pane`'s terminal and answers the child's terminal
    /// queries, unless its handle answers them itself. Returns whether the
    /// pane has a terminal to paint on.
    fn paint(&mut self, pane: PaneId, bytes: &[u8]) -> bool {
        let Some(target) = self.panes.get_mut(&pane) else {
            return false;
        };
        let keyboard_reply = target.keyboard.observe(bytes);
        let reset = target.parser.process_or_reset(bytes);
        let terminal_reply = target.parser.take_pty_writes();
        if let Some(handle) = target.handle.as_mut()
            && !handle.answers_terminal_queries()
        {
            if !keyboard_reply.is_empty() {
                let _ = handle.write_input(&keyboard_reply);
            }
            if !terminal_reply.is_empty() {
                let _ = handle.write_input(&terminal_reply);
            }
        }
        if reset {
            self.notify(NoticeLevel::Warning, Some(pane), SCREEN_RESET_NOTICE);
        }
        true
    }

    /// Respawns a pane only when its delayed restart still belongs to this
    /// generation.
    pub fn handle_respawn(&mut self, pane: PaneId, generation: SpawnGeneration) {
        if self.generations.get(&pane) == Some(&generation) {
            self.spawn(pane);
        }
    }

    /// Escalates an elapsed graceful stop only when no newer lifecycle action
    /// superseded it.
    pub fn handle_force_stop(
        &mut self,
        pane: PaneId,
        spawn_generation: SpawnGeneration,
        shutdown_generation: ShutdownGeneration,
    ) {
        if self.generations.get(&pane) != Some(&spawn_generation) {
            return;
        }
        let Some(target) = self.panes.get_mut(&pane) else {
            return;
        };
        if target.shutdown_generation != shutdown_generation
            || !target.exit_intent.awaits_force_stop()
        {
            return;
        }
        target.shutdown_generation = target.shutdown_generation.next();
        if target.unsupervised.is_some() {
            self.escalate_unsupervised(pane);
            return;
        }
        let Some(handle) = target.handle.as_mut() else {
            return;
        };
        if handle.kill().is_ok() {
            self.workspace.set_state(pane, ProcessState::Stopping);
            self.notify(NoticeLevel::Warning, Some(pane), FORCE_STOP_NOTICE);
        } else {
            target.exit_intent = target.exit_intent.force_stop_delivery_failed();
            self.notify(NoticeLevel::Error, Some(pane), STOP_DELIVERY_FAILED_NOTICE);
        }
    }

    /// The request that launches `pane`'s process as it is defined now: in
    /// its directory under the project root, with the environment of the
    /// command that last started it under its own, its stop policy, and the
    /// terminal, log and colors the current settings give a new process.
    /// `restarting` says whether the launch counts as one more restart.
    fn spawn_request(&self, pane: PaneId, restarting: bool) -> Option<SpawnRequest> {
        let process = self.workspace.process(pane)?;
        let root = self.root();
        let environment: BTreeMap<OsString, OsString> = process
            .environment()
            .vars()
            .iter()
            .map(|(name, value)| {
                (
                    OsString::from(name.as_ref()),
                    OsString::from(value.as_ref()),
                )
            })
            .collect();
        Some(
            SpawnRequest::builder()
                .process(process.name().clone())
                .command(process.command().clone())
                .working_dir(Some(process.spec().launch_dir(root)))
                .project(Some(root.as_ref().clone()))
                .launch_environment(
                    self.launch_environments
                        .get(&pane)
                        .cloned()
                        .unwrap_or_default(),
                )
                .environment(environment)
                .size(*self.settings.default_size())
                .scrollback(*self.settings.scrollback())
                .log_max_size(*self.settings.log_max_size())
                .color_scheme(*self.settings.color_scheme())
                .stop(*process.stop())
                .history(
                    SpawnHistory::builder()
                        .restarts(
                            self.restarts.get(&pane).copied().unwrap_or(0) + u32::from(restarting),
                        )
                        .last_exit(self.last_exits.get(&pane).cloned())
                        .build(),
                )
                .build(),
        )
    }

    /// Spawns `pane`'s process and connects its PTY to the service event
    /// stream.
    pub(super) fn spawn(&mut self, pane: PaneId) {
        if self.is_starting(pane) || self.refuse_unsupervised_launch(pane) {
            return;
        }
        let Some(prior_state) = self.workspace.process(pane).map(|process| *process.state()) else {
            self.notify(NoticeLevel::Error, None, SPAWN_UNKNOWN_PROCESS);
            return;
        };
        let restarting = prior_state == ProcessState::Restarting;
        let Some(request) = self.spawn_request(pane, restarting) else {
            return;
        };
        // A fresh process on this pane has a new pid, so drop the prior sample.
        self.usage.remove(&pane);
        let generation = self.bump_generation(pane);
        self.begin_run(pane);
        let sink =
            ChannelOutputSink::new(self.root().clone(), pane, generation, self.events.clone());
        self.launches.insert(
            pane,
            PendingLaunch::new(generation, prior_state, LaunchPurpose::Spawn { restarting }),
        );
        self.workspace.set_state(pane, ProcessState::Starting);
        match self.runner.detached() {
            Some(runner) => {
                let events = self.events.clone();
                let project = self.root().clone();
                thread::spawn(move || {
                    let result = runner.spawn(request, Box::new(sink));
                    let _ = events.send(ServiceEvent::Spawned {
                        project,
                        pane,
                        generation,
                        result,
                    });
                });
            },
            None => {
                let result = self.runner.spawn(request, Box::new(sink));
                self.settle_spawn(pane, generation, result);
            },
        }
    }

    /// Settles a launch on the loop: installs the new process and replays the
    /// output it already sent, or applies the failure under the restart
    /// policy. A result a later action superseded is never installed; its
    /// process is detached when a shutdown superseded it and stopped
    /// otherwise.
    pub fn handle_spawned(
        &mut self,
        pane: PaneId,
        generation: SpawnGeneration,
        result: Result<Box<dyn ProcessHandle>, PtyError>,
    ) {
        self.settle_spawn(pane, generation, result);
    }

    /// Settles the launch of `pane` at `generation` with `result`, as
    /// [`Self::handle_spawned`] describes.
    fn settle_spawn(
        &mut self,
        pane: PaneId,
        generation: SpawnGeneration,
        result: Result<Box<dyn ProcessHandle>, PtyError>,
    ) {
        let Some(launch) = self.take_launch(pane, generation) else {
            let release = self.stale_release(pane, generation);
            match (release, result) {
                (StaleRelease::Detach, Ok(handle)) => {
                    self.release_held_exit(pane);
                    release.release(handle);
                },
                (StaleRelease::Detach, Err(_)) => {
                    self.held_exits.remove(&pane);
                },
                (StaleRelease::Discard, Ok(handle)) => release.release(handle),
                (StaleRelease::Discard, Err(_)) => {},
            }
            return;
        };
        let PendingLaunch {
            purpose,
            early,
            intent,
            suspend,
            membership,
            ..
        } = launch;
        let LaunchPurpose::Spawn { restarting } = purpose else {
            if let Ok(handle) = result {
                discard(handle);
            }
            return;
        };
        let installed = self.install_spawned(pane, generation, result, restarting, early);
        self.apply_pending(pane, membership, suspend.filter(|_| installed), intent);
    }

    /// Installs a settled launch's process, or applies its failure under the
    /// restart policy. `restarting` says whether the launch is a restart.
    /// Returns whether the process was installed.
    fn install_spawned(
        &mut self,
        pane: PaneId,
        generation: SpawnGeneration,
        result: Result<Box<dyn ProcessHandle>, PtyError>,
        restarting: bool,
        early: Vec<ProcessOutput>,
    ) -> bool {
        match result {
            Ok(handle) => {
                if restarting {
                    self.count_restart(pane);
                }
                self.failing_spawns.remove(&pane);
                self.release_held_exit(pane);
                self.install_pane(pane, handle);
                let pid = self
                    .panes
                    .get(&pane)
                    .and_then(|target| target.handle.as_ref())
                    .and_then(|handle| handle.process_id());
                self.record_event(
                    Some(pane),
                    EventKind::Started,
                    pid.map(|pid| format!("pid {pid}")).unwrap_or_default(),
                );
                self.arm_probes(pane);
                self.observe_first_screen(pane);
                self.replay(pane, generation, early);
                self.runs_launch(pane, generation)
            },
            Err(error) => {
                self.deactivate(pane);
                let message = format!("{SPAWN_FAILED_NOTICE}: {error}");
                if self.failing_spawns.insert(pane) {
                    self.alert_desktop(pane, &message);
                    self.notify(NoticeLevel::Error, Some(pane), message.clone());
                }
                let next = if self
                    .workspace
                    .should_restart(pane, &ExitOutcome::FAILED_UNKNOWN)
                {
                    ProcessState::Restarting
                } else {
                    ProcessState::Crashed
                };
                self.end_run(pane, generation, ExitReason::FailedToStart, next, &message);
                self.workspace.set_state(pane, next);
                if next == ProcessState::Restarting {
                    self.schedule_restart(pane, Instant::now());
                }
                false
            },
        }
    }

    /// Whether `pane` runs the live process its launch at `generation`
    /// installed, rather than none or one a later launch replaced it with.
    pub(super) fn runs_launch(&self, pane: PaneId, generation: SpawnGeneration) -> bool {
        self.is_live(pane) && self.generations.get(&pane) == Some(&generation)
    }

    /// Records one more restart of `pane`, for `ps`.
    fn count_restart(&mut self, pane: PaneId) {
        *self.restarts.entry(pane).or_default() += 1;
    }

    /// Supervises `handle` as `pane`'s live process on a fresh terminal,
    /// painted first with the screen the process already had. The screen is
    /// a repaint, not output, so no wait or readiness check sees it.
    pub(super) fn install_pane(&mut self, pane: PaneId, mut handle: Box<dyn ProcessHandle>) {
        let screen = handle.take_initial_screen();
        let run_start = handle.run_start();
        if self.install_terminal(pane, Some(handle)) {
            self.workspace.set_state(pane, ProcessState::Running);
            if let Some(start) = run_start {
                self.place_run(pane, start);
            }
            if let Some(screen) = screen {
                self.paint(pane, &screen);
            }
        }
    }

    /// Applies the output a starting process sent before it was installed,
    /// in order.
    pub(super) fn replay(
        &mut self,
        pane: PaneId,
        generation: SpawnGeneration,
        early: Vec<ProcessOutput>,
    ) {
        for output in early {
            self.handle_output(pane, generation, output);
        }
    }

    /// Whether `pane` has a launch or adoption in flight.
    pub(super) fn is_starting(&self, pane: PaneId) -> bool {
        self.launches.contains_key(&pane)
    }

    /// Whether `pane` has a new process being launched, as opposed to a
    /// holder being adopted.
    pub(super) fn is_spawning(&self, pane: PaneId) -> bool {
        self.launches
            .get(&pane)
            .is_some_and(|launch| matches!(launch.purpose, LaunchPurpose::Spawn { .. }))
    }

    /// Launches and adoptions handed to a worker whose results have not
    /// come back yet, for a host that waits for them as it exits.
    pub fn launches_in_flight(&self) -> usize {
        self.launches.len()
    }

    /// Drops `pane`'s launch at `generation`, whose result came back after a
    /// later action or a shutdown superseded it. Returns whether it was
    /// still recorded, which only a shutdown leaves it.
    fn forget_launch(&mut self, pane: PaneId, generation: SpawnGeneration) -> bool {
        let recorded = self
            .launches
            .get(&pane)
            .is_some_and(|launch| launch.generation == generation);
        if recorded {
            self.launches.remove(&pane);
        }
        recorded
    }

    /// Forgets `pane`'s stale launch at `generation` and says how to let go
    /// of its process: detached only when a shutdown superseded that launch,
    /// so its holder keeps it for the next service; discarded when a user's
    /// stop, kill or a removal superseded it.
    pub(super) fn stale_release(
        &mut self,
        pane: PaneId,
        generation: SpawnGeneration,
    ) -> StaleRelease {
        if self.forget_launch(pane, generation) && self.detaching {
            StaleRelease::Detach
        } else {
            StaleRelease::Discard
        }
    }

    /// Records `intent` for `pane` when it is starting, to apply once its
    /// launch or adoption settles: until then there is no process to signal,
    /// and superseding the launch could leave an adopted child running behind
    /// a row that claims it stopped. Returns whether it was recorded.
    pub(super) fn record_intent(&mut self, pane: PaneId, intent: PendingIntent) -> bool {
        match self.launches.get_mut(&pane) {
            Some(launch) => {
                launch.intent = Some(intent);
                true
            },
            None => false,
        }
    }

    /// Gives `pane` the stack `membership`, or records it for the pane's
    /// launch to take when it settles if the pane is starting.
    pub(super) fn set_membership(&mut self, pane: PaneId, membership: ConfigMembership) {
        if let Some(launch) = self.launches.get_mut(&pane) {
            launch.membership = Some(membership);
        }
        if let Some(target) = self.panes.get_mut(&pane) {
            target.config_membership = membership;
        }
    }

    /// Applies what was asked of `pane` while it was starting, once its launch
    /// settled: the stack membership a stack change gave it, retiring a row
    /// removed from the stack that holds no process, then the latest pause or
    /// resume, which the caller passes only when the launch installed a live
    /// process, then the latest stop, kill or restart, each of which reaches a
    /// live, unsupervised or settled process as it would have outside a
    /// launch.
    pub(super) fn apply_pending(
        &mut self,
        pane: PaneId,
        membership: Option<ConfigMembership>,
        suspend: Option<bool>,
        intent: Option<PendingIntent>,
    ) {
        if let Some(membership) = membership {
            if let Some(target) = self.panes.get_mut(&pane) {
                target.config_membership = membership;
            }
            if membership == ConfigMembership::RetireOnExit && !self.holds_process(pane) {
                self.retire_pane(pane);
                return;
            }
        }
        match suspend {
            Some(true) => self.pause_pane(pane),
            Some(false) => self.resume_pane(pane),
            None => {},
        }
        match intent {
            Some(PendingIntent::Stop) => self.stop_pane(pane),
            Some(PendingIntent::Kill) => self.force_kill_pane(pane),
            Some(PendingIntent::Restart) => self.restart_pane(pane),
            None => {},
        }
    }

    /// Takes `pane`'s pending launch when it is the one for `generation` and
    /// nothing superseded it since; a newer launch is left in place.
    pub(super) fn take_launch(
        &mut self,
        pane: PaneId,
        generation: SpawnGeneration,
    ) -> Option<PendingLaunch> {
        let current = self.generations.get(&pane) == Some(&generation)
            && self
                .launches
                .get(&pane)
                .is_some_and(|launch| launch.generation == generation);
        if current {
            self.launches.remove(&pane)
        } else {
            None
        }
    }

    /// Gives `pane` a fresh terminal over `handle`, or over no process at all,
    /// keeping the scrollback the current settings give a new process.
    /// Returns whether the workspace has the pane's process.
    pub(super) fn install_terminal(
        &mut self,
        pane: PaneId,
        handle: Option<Box<dyn ProcessHandle>>,
    ) -> bool {
        if self.workspace.process(pane).is_none() {
            return false;
        }
        let size = handle
            .as_ref()
            .and_then(|handle| handle.screen_size())
            .unwrap_or(*self.settings.default_size());
        let started_at = handle
            .as_ref()
            .and_then(|handle| handle.started_at())
            .map_or_else(Instant::now, instant_of);
        let attached = handle
            .as_ref()
            .map_or(0, |handle| handle.attached_clients());
        self.set_pane_attached(pane, attached);
        let parser = TerminalEmulator::new(
            size.rows(),
            size.cols(),
            *self.settings.scrollback(),
            *self.settings.color_scheme(),
        );
        self.panes.insert(
            pane,
            Pane::builder()
                .parser(parser)
                .keyboard(KittyKeyboardTracker::default())
                .handle(handle)
                .unsupervised(None)
                .started_at(started_at)
                .exit_intent(ExitIntent::FollowPolicy)
                .shutdown_generation(ShutdownGeneration::initial())
                .config_membership(ConfigMembership::Tracked)
                .build(),
        );
        true
    }

    /// Invalidates pending respawns and any launch in flight, and returns the
    /// new generation.
    pub(super) fn bump_generation(&mut self, pane: PaneId) -> SpawnGeneration {
        self.launches.remove(&pane);
        self.advance_generation(pane)
    }

    /// Advances `pane`'s generation, so pending timers and output are stale,
    /// and returns it. A launch in flight is left to settle.
    pub(super) fn advance_generation(&mut self, pane: PaneId) -> SpawnGeneration {
        let entry = self
            .generations
            .entry(pane)
            .or_insert_with(SpawnGeneration::initial);
        *entry = entry.next();
        *entry
    }

    /// Invalidates the prior graceful-stop deadline for a pane.
    pub(super) fn advance_shutdown_generation(
        &mut self,
        pane: PaneId,
    ) -> Option<ShutdownGeneration> {
        let target = self.panes.get_mut(&pane)?;
        target.shutdown_generation = target.shutdown_generation.next();
        Some(target.shutdown_generation)
    }

    /// Drops `pane`'s live handle and the state that only matters while it
    /// runs: its usage sample and its attached clients, which an exit or a
    /// lost holder leaves nobody to report.
    pub(super) fn deactivate(&mut self, pane: PaneId) {
        self.usage.remove(&pane);
        self.attached.remove(&pane);
        if let Some(target) = self.panes.get_mut(&pane) {
            target.handle = None;
            target.exit_intent = ExitIntent::FollowPolicy;
        }
    }

    /// Retires a process that no longer belongs in the workspace.
    pub(super) fn retire_pane(&mut self, pane: PaneId) {
        self.end_waits(pane, None, WaitStatus::Stopped, ProcessState::Exited);
        if let Some(probes) = &self.probes {
            probes.forget(self.root(), pane);
        }
        self.taps.remove(&pane);
        self.readiness.remove(&pane);
        self.run_ends.remove(&pane);
        self.ports.remove(&pane);
        self.panes.remove(&pane);
        // Advance the spawn generation rather than dropping it. A retired pane can
        // still have a `ForceStop` or a delayed `Respawn` queued in a timer thread;
        // dropping the counter would let a process that later reuses this pane id
        // match those stale events.
        self.bump_generation(pane);
        self.restart_attempts.remove(&pane);
        self.restarts.remove(&pane);
        self.failing_spawns.remove(&pane);
        self.crash_streaks.remove(&pane);
        self.failing_screen_reads.remove(&pane);
        self.last_exits.remove(&pane);
        self.attached.remove(&pane);
        self.usage.remove(&pane);
        self.launch_environments.remove(&pane);
        self.workspace.remove(pane);
    }

    /// Applies the domain lifecycle decision to one exit, then acknowledges the
    /// exit to the finished process's handle so a holder lets go of it only
    /// once it was handled.
    fn handle_exit(&mut self, pane: PaneId, outcome: ExitOutcome) {
        let Some(finished) = self
            .panes
            .get_mut(&pane)
            .and_then(|target| target.handle.take())
        else {
            self.apply_exit(pane, outcome);
            return;
        };
        let retired = self.apply_exit(pane, outcome);
        if retired != Some(true) && self.awaits_respawn(pane) {
            if let Some(displaced) = self.held_exits.insert(pane, finished) {
                self.acknowledge(pane, displaced);
            }
        } else {
            self.acknowledge(pane, finished);
        }
    }

    /// Whether `pane` waits for its restart: scheduled, or its respawn in
    /// flight.
    fn awaits_respawn(&self, pane: PaneId) -> bool {
        self.is_spawning(pane)
            || self
                .workspace
                .process(pane)
                .is_some_and(|process| *process.state() == ProcessState::Restarting)
    }

    /// Acknowledges the exit `handle` reports, letting its holder finish; a
    /// failure is a warning, since the holder then keeps it.
    fn acknowledge(&self, pane: PaneId, mut handle: Box<dyn ProcessHandle>) {
        if handle.acknowledge_exit().is_err() {
            let known = self.workspace.process(pane).is_some();
            self.notify(
                NoticeLevel::Warning,
                known.then_some(pane),
                EXIT_ACKNOWLEDGE_FAILED_NOTICE,
            );
        }
    }

    /// Acknowledges `pane`'s exit held for a restart, once the respawn is
    /// installed or the restart will not happen.
    fn release_held_exit(&mut self, pane: PaneId) {
        if let Some(handle) = self.held_exits.remove(&pane) {
            self.acknowledge(pane, handle);
        }
    }

    /// Acknowledges every held exit whose pane no longer waits for its
    /// restart, as when a stop, a removal or a failed respawn settled it.
    /// The host runs it after each event it hands the supervisor.
    pub fn release_settled_exits(&mut self) {
        let settled: Vec<PaneId> = self
            .held_exits
            .keys()
            .copied()
            .filter(|pane| !self.awaits_respawn(*pane))
            .collect();
        for pane in settled {
            self.release_held_exit(pane);
        }
    }

    /// Applies the domain lifecycle decision to one exit. Returns whether the
    /// pane retired, or `None` when no pane was there to exit.
    pub(super) fn apply_exit(&mut self, pane: PaneId, outcome: ExitOutcome) -> Option<bool> {
        let (exit_intent, config_membership, started_at) = self.panes.get(&pane).map(|target| {
            (
                target.exit_intent,
                target.config_membership,
                target.started_at,
            )
        })?;
        let decision = ProcessLifecycle::after_exit(
            exit_intent,
            config_membership == ConfigMembership::RetireOnExit,
            self.workspace.should_restart(pane, &outcome),
            &outcome,
        );
        self.report_unrequested_exit(pane, exit_intent, started_at, &outcome);
        let next = match decision {
            ExitDecision::Retire | ExitDecision::Stop => ProcessState::Exited,
            ExitDecision::RestartNow | ExitDecision::RestartLater => ProcessState::Restarting,
            ExitDecision::Settle(state) => state,
        };
        let requested = exit_intent.is_stop() || exit_intent.is_restart();
        let generation = self.current_generation(pane);
        self.end_run(
            pane,
            generation,
            ExitReason::of(&outcome, requested),
            next,
            &outcome.to_string(),
        );
        self.last_exits.insert(pane, outcome);
        match decision {
            ExitDecision::Retire => {
                self.retire_pane(pane);
                return Some(true);
            },
            ExitDecision::Stop => {
                self.deactivate(pane);
                self.workspace.set_state(pane, ProcessState::Exited);
            },
            ExitDecision::RestartNow => {
                self.restart_attempts.remove(&pane);
                self.workspace.set_state(pane, ProcessState::Restarting);
                self.deactivate(pane);
                self.spawn(pane);
            },
            ExitDecision::RestartLater => {
                self.workspace.set_state(pane, ProcessState::Restarting);
                self.deactivate(pane);
                self.schedule_restart(pane, started_at);
            },
            ExitDecision::Settle(state) => {
                self.deactivate(pane);
                self.workspace.set_state(pane, state);
            },
        }
        Some(false)
    }

    /// Tells the user about a failed exit nobody asked for through muster, as
    /// an error notice and, when desktop notifications are on, on the
    /// desktop. Failures are told once per crash streak: a process that
    /// crash-loops under its restart policy raises one notice, not one per
    /// crash, so it never pushes every other failure out of the notices. The
    /// streak ends when a run outlasts the quick-restart window or the user
    /// starts, restarts or brings up the process.
    fn report_unrequested_exit(
        &mut self,
        pane: PaneId,
        exit_intent: ExitIntent,
        started_at: Instant,
        outcome: &ExitOutcome,
    ) {
        if started_at.elapsed() >= RESTART_STABLE_RUN {
            self.crash_streaks.remove(&pane);
        }
        if exit_intent.is_stop() || exit_intent.is_restart() || !outcome.is_failure() {
            return;
        }
        if self.workspace.process(pane).is_some() && self.crash_streaks.insert(pane) {
            let message = format!("{CRASHED_NOTICE}: {outcome}");
            self.alert_desktop(pane, &message);
            self.notify(NoticeLevel::Error, Some(pane), message);
        }
    }

    /// Raises `message` about `pane`'s process on the desktop, when desktop
    /// notifications are on and a notifier is wired.
    fn alert_desktop(&self, pane: PaneId, message: &str) {
        if !*self.settings.desktop_notifications() {
            return;
        }
        let (Some(notifier), Some(process)) = (&self.notifier, self.workspace.process(pane)) else {
            return;
        };
        notifier.notify(
            &Notification::builder()
                .pane(pane)
                .source(process.name().clone())
                .body(Some(message.to_string()))
                .build(),
        );
    }

    /// Schedules a backoff restart and tags it with the current generation.
    fn schedule_restart(&mut self, pane: PaneId, started_at: Instant) {
        let attempts = if started_at.elapsed() >= RESTART_STABLE_RUN {
            0
        } else {
            self.restart_attempts.get(&pane).copied().unwrap_or(0)
        };
        let delay = (RESTART_BACKOFF_BASE * 2u32.pow(attempts.min(RESTART_BACKOFF_MAX_EXP)))
            .min(RESTART_BACKOFF_MAX);
        self.restart_attempts.insert(pane, attempts + 1);
        let generation = self.bump_generation(pane);
        let sender = self.events.clone();
        let project = self.root().clone();
        thread::spawn(move || {
            thread::sleep(delay);
            let _ = sender.send(ServiceEvent::Respawn {
                project,
                pane,
                generation,
            });
        });
    }

    /// Schedules escalation after a graceful stop's configured deadline.
    pub(super) fn schedule_force_stop(
        &self,
        pane: PaneId,
        spawn_generation: SpawnGeneration,
        shutdown_generation: ShutdownGeneration,
        grace_period: Duration,
    ) {
        let sender = self.events.clone();
        let project = self.root().clone();
        thread::spawn(move || {
            thread::sleep(grace_period);
            let _ = sender.send(ServiceEvent::ForceStop {
                project,
                pane,
                spawn_generation,
                shutdown_generation,
            });
        });
    }

    /// Sends the stop policy's graceful signal and schedules a single matching
    /// escalation; a signal that cannot be delivered falls back to a kill.
    pub(super) fn request_graceful_transition(
        &mut self,
        pane: PaneId,
        request: impl FnOnce(ExitIntent) -> ExitIntent,
        restart: bool,
    ) {
        let Some(policy) = self.stop_policy_of(pane) else {
            return;
        };
        let grace_period = policy.grace_period().into_inner();
        let spawn_generation = self.generations.get(&pane).copied();
        let shutdown_generation = self.advance_shutdown_generation(pane);
        let mut awaiting_grace = false;
        let mut used_fallback = false;
        let mut delivered = false;
        if let Some(target) = self.panes.get_mut(&pane) {
            target.exit_intent = request(target.exit_intent);
            if let Some(handle) = target.handle.as_mut() {
                delivered = match handle.terminate(*policy.signal(), grace_period) {
                    Ok(()) => {
                        awaiting_grace = true;
                        true
                    },
                    Err(_) => {
                        used_fallback = true;
                        handle.kill().is_ok()
                    },
                };
                target.exit_intent = match (restart, delivered) {
                    (true, true) => target.exit_intent.restart_delivered(),
                    (true, false) => target.exit_intent.restart_delivery_failed(),
                    (false, true) => target.exit_intent.stop_delivered(),
                    (false, false) => target.exit_intent.stop_delivery_failed(),
                };
            }
        }
        if delivered {
            self.workspace.set_state(pane, ProcessState::Stopping);
            if used_fallback {
                self.notify(
                    NoticeLevel::Warning,
                    Some(pane),
                    GRACEFUL_STOP_FALLBACK_NOTICE,
                );
            }
        } else {
            self.notify(NoticeLevel::Error, Some(pane), STOP_DELIVERY_FAILED_NOTICE);
        }
        if let (true, Some(spawn_generation), Some(shutdown_generation)) =
            (awaiting_grace, spawn_generation, shutdown_generation)
        {
            self.schedule_force_stop(pane, spawn_generation, shutdown_generation, grace_period);
        }
    }

    /// `pane`'s stop policy, when the workspace has its process.
    pub(super) fn stop_policy_of(&self, pane: PaneId) -> Option<StopPolicy> {
        self.workspace.process(pane).map(|process| *process.stop())
    }

    /// The panes marked to start with the project and not already running.
    fn autostart_panes(&self) -> Vec<PaneId> {
        self.workspace
            .processes()
            .iter()
            .filter(|process| *process.autostart())
            .map(|process| *process.id())
            .filter(|pane| !self.is_live(*pane))
            .collect()
    }
}

/// Lets go of a process whose launch or adoption a later action superseded,
/// stopping it so nothing is left running unsupervised: a holder is reached
/// afresh, its child killed and the exit collected, off the loop, so the
/// holder finishes; any other process is killed.
pub(super) fn discard(mut handle: Box<dyn ProcessHandle>) {
    match handle.holder_socket().map(Path::to_path_buf) {
        Some(socket) => {
            drop(handle);
            thread::spawn(move || abandon(&socket));
        },
        None => {
            let _ = handle.kill();
        },
    }
}

/// The instant that was `started` on the wall clock, so a process that started
/// before this service keeps its uptime; one in the future reads as now.
fn instant_of(started: SystemTime) -> Instant {
    let elapsed = SystemTime::now()
        .duration_since(started)
        .unwrap_or_default();
    Instant::now()
        .checked_sub(elapsed)
        .unwrap_or_else(Instant::now)
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        collections::VecDeque,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use super::*;
    use crate::{
        adapter::service::{
            LastExit, ProcessUsage,
            supervisor::{entity::UNKNOWN_PROCESS_NOTICE, testing::*},
        },
        domain::{
            config::{Environment, ProcessSpec},
            port::{OutputSink, ProcessRunner},
            process::{Process, RestartPolicy, StopSignal},
            pty::{ExitCause, PtySize},
            value::{Cols, EnvName, EnvValue, Rows},
        },
    };

    const PANE: u64 = 0;

    /// The test process's pane.
    fn pane() -> PaneId {
        PaneId::new(PANE)
    }

    /// The test process's name.
    fn p() -> ProcessName {
        name("p")
    }

    /// The test process `p` on [`PANE`], under `restart`.
    fn process(restart: RestartPolicy) -> Process {
        spec("p", "true", restart).to_process(pane())
    }

    /// No environment of the client's own.
    fn no_env() -> LaunchEnvironment {
        LaunchEnvironment::default()
    }

    /// A harness over `process` whose autostart set has been brought up.
    fn started(runner: Box<dyn ProcessRunner>, process: Process) -> Harness {
        let mut harness = Harness::new(runner, vec![process]);
        harness.up();
        harness
    }

    /// A running process with `restart` policy on a runner that never fails.
    fn running(restart: RestartPolicy) -> Harness {
        started(Box::new(FakeRunner), process(restart))
    }

    /// A live process whose handle records graceful and hard stops.
    fn with_stop_signals() -> (Harness, Arc<StopSignals>) {
        let signals = Arc::new(StopSignals::default());
        let harness = started(
            Box::new(StopSignalRunner {
                signals: signals.clone(),
            }),
            process(RestartPolicy::Never),
        );
        (harness, signals)
    }

    /// A live process whose first hard-stop delivery fails and second succeeds.
    fn with_retrying_stop_signals(
        terminate_outcome: TerminateOutcome,
        restart: RestartPolicy,
    ) -> (Harness, Arc<RetryStopSignals>) {
        let signals = Arc::new(RetryStopSignals::default());
        let harness = started(
            Box::new(RetryStopRunner {
                signals: signals.clone(),
                terminate_outcome,
            }),
            process(restart),
        );
        (harness, signals)
    }

    /// The test process's lifecycle state.
    fn state(harness: &Harness) -> ProcessState {
        *harness
            .supervisor
            .workspace()
            .process(pane())
            .unwrap()
            .state()
    }

    /// The test pane's current spawn generation.
    fn current_gen(harness: &Harness) -> SpawnGeneration {
        *harness.supervisor.generations().get(&pane()).unwrap()
    }

    /// The test pane's current graceful-stop generation.
    fn current_shutdown_gen(harness: &Harness) -> ShutdownGeneration {
        harness.supervisor.panes()[&pane()].shutdown_generation
    }

    /// What the test pane's next exit means.
    fn exit_intent(harness: &Harness) -> ExitIntent {
        harness.supervisor.panes()[&pane()].exit_intent
    }

    /// Whether the test pane has a live process.
    fn is_live(harness: &Harness) -> bool {
        harness.supervisor.is_live(pane())
    }

    /// Delivers `outcome` as the current process's exit.
    fn exit(harness: &mut Harness, outcome: ExitOutcome) {
        let generation = current_gen(harness);
        harness
            .supervisor
            .handle_output(pane(), generation, ProcessOutput::Exited(outcome));
    }

    /// The levels of `harness`'s notices, with their messages.
    fn notice_levels(harness: &Harness) -> Vec<(NoticeLevel, String)> {
        harness
            .notices
            .lock()
            .unwrap()
            .recent(harness.supervisor.root())
            .iter()
            .map(|notice| (*notice.level(), notice.message().clone()))
            .collect()
    }

    /// The client environment `KEY=value`.
    fn client_env(key: &str, value: &str) -> LaunchEnvironment {
        LaunchEnvironment::from_vars([(key.into(), value.into())])
    }

    /// Retiring a pane drops its metrics sample, so a later process that reuses
    /// the pane id does not briefly show the retired process's usage.
    #[test]
    fn retiring_a_pane_drops_its_usage_sample() {
        let mut harness = running(RestartPolicy::Never);
        harness.supervisor.usage.insert(
            pane(),
            ProcessUsage::builder()
                .pid(4242)
                .cpu_percent(3.0)
                .memory_bytes(1024)
                .build(),
        );

        harness.supervisor.retire_pane(pane());

        assert!(!harness.supervisor.usage().contains_key(&pane()));
    }

    /// A start for a pane with no process entity is refused with a notice
    /// before any side effect.
    #[test]
    fn spawning_a_pane_without_a_process_reports_it_and_changes_nothing() {
        let spawns = SpawnRecorder::default();
        let mut harness = Harness::new(
            Box::new(RequestRecordingRunner {
                recorder: spawns.clone(),
            }),
            Vec::new(),
        );

        harness.supervisor.spawn(pane());

        assert_eq!(harness.notice_messages(), vec![
            SPAWN_UNKNOWN_PROCESS.to_string()
        ]);
        assert!(spawns.requests.borrow().is_empty());
        assert!(!harness.supervisor.generations().contains_key(&pane()));
        assert!(!harness.supervisor.panes().contains_key(&pane()));
    }

    /// A launch carries the process's definition, the client's environment
    /// under the process's own, its stop policy and the settings' terminal,
    /// log and colors; a policy restart keeps the client's environment and a
    /// user's restart takes the new client's.
    #[test]
    fn a_launch_carries_the_definition_the_client_environment_and_the_settings() {
        let spawns = SpawnRecorder::default();
        let own = spec("p", "npm run dev", RestartPolicy::Always)
            .with_working_dir(Some("web".into()))
            .with_environment(Environment::empty().with_var(
                EnvName::try_new("PORT").unwrap(),
                EnvValue::try_new("5173").unwrap(),
            ));
        let mut harness = Harness::over(
            Box::new(RequestRecordingRunner {
                recorder: spawns.clone(),
            }),
            &[own],
        );

        harness.supervisor.up(&client_env("FROM", "first"));
        exit(&mut harness, ExitOutcome::FAILED_UNKNOWN);
        harness
            .supervisor
            .handle_respawn(pane(), current_gen(&harness));
        harness
            .supervisor
            .restart(&p(), &client_env("FROM", "second"));

        let requests = spawns.requests.borrow();
        let first = &requests[0];
        assert_eq!(first.command().as_ref(), "npm run dev");
        assert_eq!(
            first.working_dir().as_deref(),
            Some(Path::new(TEST_PROJECT).join("web").as_path())
        );
        assert_eq!(first.project().as_deref(), Some(Path::new(TEST_PROJECT)));
        assert_eq!(
            first.environment(),
            &BTreeMap::from([(OsString::from("PORT"), OsString::from("5173"))])
        );
        assert_eq!(first.stop(), test_stop());
        let settings = test_settings();
        assert_eq!(first.size(), *settings.default_size());
        assert_eq!(first.scrollback(), *settings.scrollback());
        assert_eq!(first.log_max_size(), *settings.log_max_size());
        let launched_with = |index: usize| requests[index].launch_environment().clone();
        let from = |value: &str| BTreeMap::from([(OsString::from("FROM"), OsString::from(value))]);
        assert_eq!(launched_with(0), from("first"));
        assert_eq!(launched_with(1), from("first"), "a policy restart keeps it");
        assert_eq!(requests.len(), 2, "the restart waits for the exit");
        drop(requests);
        exit(&mut harness, ExitOutcome::Succeeded);
        assert_eq!(
            spawns.requests.borrow()[2].launch_environment(),
            &from("second"),
            "a user's restart takes the asking client's"
        );
    }

    /// New settings reach the next launch only: a running process keeps the
    /// terminal it started with.
    #[test]
    fn new_settings_change_the_next_launch_only() {
        let spawns = SpawnRecorder::default();
        let mut harness = Harness::over(
            Box::new(RequestRecordingRunner {
                recorder: spawns.clone(),
            }),
            &[
                spec("p", "true", RestartPolicy::Never),
                spec("q", "true", RestartPolicy::Never).with_autostart(false),
            ],
        );
        harness.up();
        let bigger = PtySize::builder()
            .rows(Rows::new(40))
            .cols(Cols::new(120))
            .build();

        harness
            .supervisor
            .apply_settings(&test_settings().with_default_size(bigger));
        harness.supervisor.start(&name("q"), &no_env());

        let requests = spawns.requests.borrow();
        assert_eq!(requests[0].size(), *test_settings().default_size());
        assert_eq!(requests[1].size(), bigger);
        assert_eq!(
            harness.supervisor.panes()[&pane()].parser().size(),
            (24, 24),
            "the running process keeps its terminal"
        );
    }

    #[test]
    fn exit_without_restart_keeps_the_terminal_and_marks_crashed() {
        let mut harness = running(RestartPolicy::Never);

        exit(&mut harness, ExitOutcome::FAILED_UNKNOWN);

        assert!(
            harness.supervisor.panes().contains_key(&pane()),
            "the terminal is retained"
        );
        assert!(!is_live(&harness), "the live handle is dropped");
        assert_eq!(state(&harness), ProcessState::Crashed);
    }

    /// An exit or a lost holder leaves nobody to report attached clients, so
    /// `ps` stops counting them.
    #[test]
    fn an_exit_clears_the_attached_count() {
        let mut harness = running(RestartPolicy::Never);
        harness.supervisor.set_attached(&p(), 2);
        let generation = current_gen(&harness);

        exit(&mut harness, ExitOutcome::Succeeded);
        harness
            .supervisor
            .handle_output(pane(), generation, ProcessOutput::Attached(1));

        assert_eq!(*harness.supervisor.snapshot().processes()[0].attached(), 0);
    }

    #[test]
    fn spawn_failure_under_auto_restart_stays_in_backoff() {
        let harness = started(Box::new(FailingRunner), process(RestartPolicy::Always));

        assert_eq!(state(&harness), ProcessState::Restarting);
        assert!(
            harness.supervisor.restart_attempts.contains_key(&pane()),
            "a backoff retry must be scheduled after a failed spawn"
        );
    }

    #[test]
    fn spawn_failure_without_restart_is_crashed() {
        let harness = started(Box::new(FailingRunner), process(RestartPolicy::Never));

        assert_eq!(state(&harness), ProcessState::Crashed);
    }

    #[test]
    fn pause_then_resume_toggles_state_while_the_process_stays_alive() {
        let mut harness = running(RestartPolicy::Never);

        harness.supervisor.pause(&p());
        assert_eq!(state(&harness), ProcessState::Paused);
        assert!(is_live(&harness), "a paused process keeps its live handle");

        harness.supervisor.resume(&p());
        assert_eq!(state(&harness), ProcessState::Running);
    }

    #[test]
    fn pausing_a_finished_process_is_a_noop() {
        let mut harness = running(RestartPolicy::Never);
        harness.supervisor.stop(&p());
        exit(&mut harness, ExitOutcome::Succeeded);
        assert_eq!(state(&harness), ProcessState::Exited);

        harness.supervisor.pause(&p());

        assert_eq!(state(&harness), ProcessState::Exited);
    }

    #[test]
    fn a_failed_pause_signal_leaves_the_state_unchanged() {
        let mut harness = started(Box::new(PauseFailRunner), process(RestartPolicy::Never));

        harness.supervisor.pause(&p());

        assert_eq!(state(&harness), ProcessState::Running);
        assert_eq!(harness.notice_messages(), vec![
            PAUSE_FAILED_NOTICE.to_string()
        ]);
    }

    #[test]
    fn user_stop_is_exited_not_crashed_even_on_nonzero_exit() {
        let mut harness = running(RestartPolicy::Always);
        harness.supervisor.stop(&p());

        exit(&mut harness, ExitOutcome::FAILED_UNKNOWN);

        assert_eq!(state(&harness), ProcessState::Exited);
        assert!(!is_live(&harness));
        assert!(
            harness.notice_messages().is_empty(),
            "an exit the user asked for is no crash"
        );
    }

    /// A failed exit nobody asked for raises an error notice naming how the
    /// process ended, once per crash streak, and `ps` shows that exit while
    /// the process is not running.
    #[test]
    fn a_crash_loop_raises_one_notice_until_its_streak_ends() {
        let crash = || ExitOutcome::Failed(ExitCause::Status(1));
        let mut harness = running(RestartPolicy::Always);

        for _ in 0..5 {
            harness.supervisor.apply_exit(pane(), crash());
        }
        assert_eq!(
            harness.notice_messages(),
            vec!["crashed: exited with status 1".to_string()],
            "one notice for the whole loop"
        );
        assert_eq!(
            harness.supervisor.snapshot().processes()[0]
                .last_exit()
                .as_ref()
                .map(LastExit::outcome),
            Some(&crash()),
            "ps still shows how it last ended"
        );

        harness.supervisor.start(&p(), &no_env());
        harness.supervisor.apply_exit(pane(), crash());
        assert_eq!(
            harness.notice_messages().len(),
            2,
            "a start by the user begins a new streak"
        );

        harness
            .supervisor
            .panes
            .get_mut(&pane())
            .unwrap()
            .started_at = Instant::now() - RESTART_STABLE_RUN;
        harness.supervisor.apply_exit(pane(), crash());
        assert_eq!(
            harness.notice_messages().len(),
            3,
            "a run past the quick-restart window ends the streak"
        );
    }

    /// A crash reaches the desktop while desktop notifications are on, once
    /// per streak, and stays a notice only once they are off.
    #[test]
    fn a_crash_reaches_the_desktop_only_while_desktop_notifications_are_on() {
        let notifier = RecordingNotifier::default();
        let mut harness = running(RestartPolicy::Always);
        harness.supervisor.set_notifier(Box::new(notifier.clone()));
        harness
            .supervisor
            .apply_settings(&test_settings().with_desktop_notifications(true));

        for _ in 0..3 {
            harness
                .supervisor
                .apply_exit(pane(), ExitOutcome::Failed(ExitCause::Status(1)));
        }
        let delivered = notifier.delivered();
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].source(), &p());
        assert_eq!(
            delivered[0].body().as_deref(),
            Some("crashed: exited with status 1")
        );

        harness.supervisor.apply_settings(&test_settings());
        harness.supervisor.start(&p(), &no_env());
        harness
            .supervisor
            .apply_exit(pane(), ExitOutcome::Failed(ExitCause::Status(1)));
        assert_eq!(notifier.delivered().len(), 1);
        assert_eq!(harness.notice_messages().len(), 2);
    }

    /// `up`, like a start, begins a new crash streak, so a process that
    /// crashes again after it is told again.
    #[test]
    fn up_begins_a_new_crash_streak() {
        let crash = || ExitOutcome::Failed(ExitCause::Status(1));
        let mut harness = running(RestartPolicy::Never);
        harness.supervisor.apply_exit(pane(), crash());

        harness.up();
        harness.supervisor.apply_exit(pane(), crash());

        assert_eq!(harness.notice_messages().len(), 2);
    }

    #[test]
    fn a_crash_is_noticed_with_its_exit_and_shown_by_ps() {
        for policy in [
            RestartPolicy::Never,
            RestartPolicy::OnFailure,
            RestartPolicy::Always,
        ] {
            let mut harness = running(policy);

            exit(&mut harness, ExitOutcome::Failed(ExitCause::Status(3)));

            assert_eq!(
                notice_levels(&harness),
                vec![(
                    NoticeLevel::Error,
                    "crashed: exited with status 3".to_string()
                )],
                "{policy:?}"
            );
            let status = harness.supervisor.snapshot();
            assert_eq!(
                status.processes()[0]
                    .last_exit()
                    .as_ref()
                    .map(LastExit::outcome),
                Some(&ExitOutcome::Failed(ExitCause::Status(3))),
                "{policy:?}"
            );
        }
    }

    /// A clean exit is no crash, and `ps` still says how the process ended.
    #[test]
    fn a_clean_exit_raises_no_notice_but_is_shown() {
        let mut harness = running(RestartPolicy::Never);

        exit(&mut harness, ExitOutcome::Succeeded);

        assert!(harness.notice_messages().is_empty());
        let shown = harness.supervisor.snapshot().processes()[0]
            .last_exit()
            .clone();
        assert_eq!(
            shown.as_ref().map(LastExit::outcome),
            Some(&ExitOutcome::Succeeded)
        );
        assert_eq!(shown.map(|exit| exit.reason()), Some(ExitReason::Exited));
    }

    /// A stop sends the policy's signal once, enters stopping, and escalates.
    #[test]
    fn repeated_manual_stop_reuses_one_graceful_escalation() {
        let (mut harness, signals) = with_stop_signals();
        harness.supervisor.stop(&p());
        harness.supervisor.stop(&p());

        assert_eq!(signals.terminates.load(Ordering::SeqCst), 1);
        assert_eq!(signals.kills.load(Ordering::SeqCst), 0);
        assert_eq!(
            *signals.termination.lock().unwrap(),
            Some((StopSignal::Interrupt, TEST_STOP_GRACE))
        );
        assert_eq!(state(&harness), ProcessState::Stopping);

        let (spawn, shutdown) = (current_gen(&harness), current_shutdown_gen(&harness));
        harness
            .supervisor
            .handle_force_stop(pane(), spawn, shutdown);
        assert_eq!(signals.kills.load(Ordering::SeqCst), 1);
        assert_eq!(harness.notice_messages(), vec![
            FORCE_STOP_NOTICE.to_string()
        ]);
    }

    /// Retiring advances the spawn generation, so a stale `ForceStop` queued
    /// before the retirement never matches a process that reuses the pane id.
    #[test]
    fn reusing_a_retired_pane_id_advances_past_a_queued_force_stop() {
        let (mut harness, _signals) = with_stop_signals();
        let scheduled = current_gen(&harness);

        harness.supervisor.retire_pane(pane());
        let reused = harness.supervisor.bump_generation(pane());

        assert_ne!(reused, scheduled);
        assert!(reused.into_inner() > scheduled.into_inner());
    }

    /// Retiring must advance the spawn generation, not merely keep it, so a
    /// stale respawn never starts whatever reuses the pane id.
    #[test]
    fn retiring_a_pane_voids_a_delayed_respawn_on_the_reused_id() {
        let (mut harness, _signals) = with_stop_signals();
        let scheduled = current_gen(&harness);

        harness.supervisor.retire_pane(pane());

        assert_ne!(
            harness.supervisor.generations().get(&pane()).copied(),
            Some(scheduled)
        );
        harness.supervisor.handle_respawn(pane(), scheduled);
        assert!(!harness.supervisor.panes().contains_key(&pane()));
    }

    /// Force-kill ignores the graceful stop policy.
    #[test]
    fn force_stop_bypasses_the_graceful_policy() {
        let (mut harness, signals) = with_stop_signals();

        harness.supervisor.force_kill(&p());

        assert_eq!(signals.terminates.load(Ordering::SeqCst), 0);
        assert_eq!(signals.kills.load(Ordering::SeqCst), 1);
        assert_eq!(state(&harness), ProcessState::Stopping);
    }

    /// Restart waits for the graceful exit before spawning a new child.
    #[test]
    fn restart_uses_the_graceful_policy_then_respawns() {
        let (mut harness, signals) = with_stop_signals();

        harness.supervisor.restart(&p(), &no_env());

        assert_eq!(signals.terminates.load(Ordering::SeqCst), 1);
        assert_eq!(signals.kills.load(Ordering::SeqCst), 0);
        assert_eq!(state(&harness), ProcessState::Stopping);
        assert_eq!(exit_intent(&harness), ExitIntent::RestartInFlight);

        exit(&mut harness, ExitOutcome::Succeeded);

        assert_eq!(state(&harness), ProcessState::Running);
        assert!(is_live(&harness));
    }

    /// Superseding a graceful stop gives the replacement restart its full grace
    /// period by making the earlier escalation event stale.
    #[test]
    fn a_new_graceful_action_invalidates_the_previous_deadline() {
        let (mut harness, signals) = with_stop_signals();
        let spawn_generation = current_gen(&harness);

        harness.supervisor.stop(&p());
        let stale_shutdown = current_shutdown_gen(&harness);
        harness.supervisor.restart(&p(), &no_env());
        let current_shutdown = current_shutdown_gen(&harness);

        assert_ne!(stale_shutdown, current_shutdown);
        harness
            .supervisor
            .handle_force_stop(pane(), spawn_generation, stale_shutdown);
        assert_eq!(signals.kills.load(Ordering::SeqCst), 0);
        assert_eq!(exit_intent(&harness), ExitIntent::RestartInFlight);

        harness
            .supervisor
            .handle_force_stop(pane(), spawn_generation, current_shutdown);
        assert_eq!(signals.kills.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_stop_can_be_retried_after_both_signals_fail() {
        let (mut harness, signals) =
            with_retrying_stop_signals(TerminateOutcome::Fails, RestartPolicy::Never);

        harness.supervisor.stop(&p());
        assert_eq!(exit_intent(&harness), ExitIntent::StopRetryable);

        harness.supervisor.stop(&p());
        harness.supervisor.stop(&p());

        assert_eq!(exit_intent(&harness), ExitIntent::StopInFlight);
        assert_eq!(signals.terminates.load(Ordering::SeqCst), 2);
        assert_eq!(signals.kills.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn a_stop_can_be_retried_after_escalation_fails() {
        let (mut harness, signals) =
            with_retrying_stop_signals(TerminateOutcome::Succeeds, RestartPolicy::Never);

        harness.supervisor.stop(&p());
        let (spawn, shutdown) = (current_gen(&harness), current_shutdown_gen(&harness));
        harness
            .supervisor
            .handle_force_stop(pane(), spawn, shutdown);
        assert_eq!(exit_intent(&harness), ExitIntent::StopRetryable);

        harness.supervisor.stop(&p());
        let (spawn, shutdown) = (current_gen(&harness), current_shutdown_gen(&harness));
        harness
            .supervisor
            .handle_force_stop(pane(), spawn, shutdown);

        assert_eq!(exit_intent(&harness), ExitIntent::StopInFlight);
        assert_eq!(signals.terminates.load(Ordering::SeqCst), 2);
        assert_eq!(signals.kills.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn failed_stop_delivery_still_suppresses_restart_when_the_child_exits() {
        let (mut harness, _signals) =
            with_retrying_stop_signals(TerminateOutcome::Fails, RestartPolicy::Always);

        harness.supervisor.stop(&p());
        exit(&mut harness, ExitOutcome::FAILED_UNKNOWN);

        assert_eq!(state(&harness), ProcessState::Exited);
        assert!(!is_live(&harness));
    }

    /// Stop sends an async signal and a restart lands before the exit: the
    /// newer restart wins. A never policy means only the forced restart can
    /// respawn.
    #[test]
    fn restart_after_stop_wins_before_the_exit_arrives() {
        let mut harness = running(RestartPolicy::Never);
        harness.supervisor.stop(&p());
        harness.supervisor.restart(&p(), &no_env());

        exit(&mut harness, ExitOutcome::FAILED_UNKNOWN);

        assert_eq!(state(&harness), ProcessState::Running);
        assert!(is_live(&harness), "the process was respawned");
    }

    #[test]
    fn stale_respawn_after_manual_restart_is_ignored() {
        let mut harness = running(RestartPolicy::Always);
        exit(&mut harness, ExitOutcome::FAILED_UNKNOWN);
        let stale = current_gen(&harness);
        assert!(!is_live(&harness));

        harness.supervisor.restart(&p(), &no_env());
        let current = current_gen(&harness);
        assert_ne!(current, stale);
        assert!(is_live(&harness));

        harness.supervisor.handle_respawn(pane(), stale);
        assert_eq!(current_gen(&harness), current);
        assert!(is_live(&harness));
    }

    #[test]
    fn failed_forced_respawn_deactivates_the_pane() {
        let mut harness = started(
            Box::new(FlakyRunner {
                spawns: AtomicUsize::new(0),
            }),
            process(RestartPolicy::Never),
        );
        assert!(is_live(&harness));

        harness.supervisor.restart(&p(), &no_env());
        exit(&mut harness, ExitOutcome::FAILED_UNKNOWN);

        assert!(!is_live(&harness), "a failed respawn leaves no dead handle");
        assert_eq!(state(&harness), ProcessState::Crashed);
    }

    #[test]
    fn output_from_a_superseded_generation_is_discarded() {
        let mut harness = running(RestartPolicy::Never);
        let old_gen = current_gen(&harness);

        harness.supervisor.restart(&p(), &no_env());
        harness.supervisor.handle_output(
            pane(),
            old_gen,
            ProcessOutput::Exited(ExitOutcome::FAILED_UNKNOWN),
        );
        let new_gen = current_gen(&harness);
        assert_ne!(new_gen, old_gen);

        harness
            .supervisor
            .handle_output(pane(), old_gen, ProcessOutput::Chunk(b"stale".to_vec()));
        harness
            .supervisor
            .handle_output(pane(), new_gen, ProcessOutput::Chunk(b"fresh".to_vec()));

        let screen = harness.supervisor.panes()[&pane()].parser.screen_text();
        assert!(screen.contains("fresh"));
        assert!(!screen.contains("stale"));
    }

    /// An output stream that broke is reported as an error naming the process.
    /// With no holder to reconnect to and no child pid to find, the failed
    /// reconnect is reported too and settles as an exit under the policy,
    /// rather than leaving a running row with nothing reading it, and that
    /// exit, its cause unseen, is still told as a crash.
    #[test]
    fn an_unreadable_output_stream_is_a_notice_naming_the_process() {
        let mut harness = running(RestartPolicy::Always);

        harness.supervisor.handle_output(
            pane(),
            current_gen(&harness),
            ProcessOutput::Unreadable("bad frame".to_string()),
        );

        let notices = harness
            .notices
            .lock()
            .unwrap()
            .recent(harness.supervisor.root());
        assert_eq!(
            notices.len(),
            3,
            "the break, the failed reconnect and the crash"
        );
        assert_eq!(notices[0].process(), &Some(p()));
        assert_eq!(*notices[0].level(), NoticeLevel::Error);
        assert!(notices[0].message().contains("bad frame"));
        assert_eq!(
            notices[2].message(),
            &format!("{CRASHED_NOTICE}: {}", ExitOutcome::FAILED_UNKNOWN)
        );
        assert_eq!(state(&harness), ProcessState::Restarting);
        assert!(!harness.supervisor.is_live(pane()));
    }

    /// A running process is not started a second time over its live handle.
    #[test]
    fn starting_a_running_process_leaves_it_alone() {
        let spawns = SpawnRecorder::default();
        let mut harness = started(
            Box::new(RequestRecordingRunner {
                recorder: spawns.clone(),
            }),
            process(RestartPolicy::Never),
        );
        let generation = current_gen(&harness);

        harness.supervisor.start(&p(), &no_env());

        assert_eq!(spawns.requests.borrow().len(), 1);
        assert_eq!(current_gen(&harness), generation);
    }

    /// Shutting down lets go of every process without killing it, so a holder
    /// keeps its process for the next service, and leaves late timers stale.
    #[test]
    fn shutdown_leaves_every_handle_unkilled() {
        let kills = Arc::new(AtomicUsize::new(0));
        let mut harness = Harness::over(
            Box::new(KillCountRunner {
                kills: kills.clone(),
            }),
            &[
                spec("p", "true", RestartPolicy::Always),
                spec("q", "true", RestartPolicy::Never),
            ],
        );
        harness.up();
        let generation = current_gen(&harness);

        harness.supervisor.shutdown();

        assert_eq!(kills.load(Ordering::SeqCst), 0, "no process was killed");
        assert!(!harness.supervisor.is_live(pane()));
        assert!(!harness.supervisor.is_live(PaneId::new(1)));
        harness.supervisor.handle_respawn(pane(), generation);
        assert!(
            !harness.supervisor.is_live(pane()),
            "a respawn queued before shutdown starts nothing"
        );

        let mut failing = started(Box::new(FailingRunner), process(RestartPolicy::Always));
        let pending = current_gen(&failing);
        failing.supervisor.shutdown();
        let after_shutdown = current_gen(&failing);
        failing.supervisor.handle_respawn(pane(), pending);
        assert_eq!(
            current_gen(&failing),
            after_shutdown,
            "a respawn queued for a process that never started spawns nothing"
        );
    }

    /// Restarting a stopped process counts as a restart, as does restarting a
    /// live one.
    #[test]
    fn restarts_on_request_are_counted_whether_or_not_the_process_runs() {
        let mut harness = running(RestartPolicy::Never);
        harness.supervisor.stop(&p());
        exit(&mut harness, ExitOutcome::Succeeded);

        harness.supervisor.restart(&p(), &no_env());
        assert_eq!(*harness.supervisor.snapshot().processes()[0].restarts(), 1);

        harness.supervisor.restart(&p(), &no_env());
        exit(&mut harness, ExitOutcome::Succeeded);
        assert_eq!(*harness.supervisor.snapshot().processes()[0].restarts(), 2);
    }

    /// A runner whose spawns succeed or fail in a scripted order.
    struct ScriptedRunner {
        outcomes: RefCell<VecDeque<bool>>,
    }

    impl ProcessRunner for ScriptedRunner {
        fn spawn(
            &self,
            _request: SpawnRequest,
            _sink: Box<dyn OutputSink>,
        ) -> Result<Box<dyn ProcessHandle>, PtyError> {
            if self.outcomes.borrow_mut().pop_front().unwrap_or(false) {
                Ok(Box::new(FakeHandle))
            } else {
                Err(PtyError::System("spawn failed".to_string()))
            }
        }
    }

    /// A process that keeps failing to spawn under an auto-restart policy is
    /// reported once per failure streak, not on every backoff retry.
    #[test]
    fn a_failing_spawn_is_reported_once_per_streak() {
        let mut harness = started(
            Box::new(ScriptedRunner {
                outcomes: RefCell::new(VecDeque::from([false, false, true, false])),
            }),
            process(RestartPolicy::Always),
        );
        let retry = current_gen(&harness);
        harness.supervisor.handle_respawn(pane(), retry);
        let retry = current_gen(&harness);
        harness.supervisor.handle_respawn(pane(), retry);
        assert!(is_live(&harness));
        exit(&mut harness, ExitOutcome::FAILED_UNKNOWN);
        let retry = current_gen(&harness);
        harness.supervisor.handle_respawn(pane(), retry);

        let reports = |harness: &Harness| {
            harness
                .notice_messages()
                .iter()
                .filter(|message| message.starts_with(SPAWN_FAILED_NOTICE))
                .count()
        };
        assert_eq!(
            reports(&harness),
            2,
            "one report per streak, the second after a success"
        );

        harness.supervisor.start(&p(), &no_env());
        assert_eq!(
            reports(&harness),
            3,
            "a start the user asks for reports its own failure"
        );
    }

    /// `up` starts exactly the autostart set and skips a process that is
    /// already running.
    #[test]
    fn up_starts_the_autostart_set_once() {
        let spawns = SpawnRecorder::default();
        let mut harness = Harness::over(
            Box::new(RequestRecordingRunner {
                recorder: spawns.clone(),
            }),
            &[
                spec("p", "true", RestartPolicy::Never),
                spec("manual", "true", RestartPolicy::Never).with_autostart(false),
            ],
        );

        let first = harness.supervisor.up(&no_env());
        let second = harness.supervisor.up(&no_env());

        let started: Vec<String> = spawns
            .requests
            .borrow()
            .iter()
            .map(|request| request.process().as_ref().to_string())
            .collect();
        assert_eq!(started, vec!["p".to_string()]);
        assert_eq!(first.acted(), &vec![p()]);
        assert!(second.acted().is_empty());
        assert_eq!(state(&harness), ProcessState::Running);
        assert_eq!(
            *harness
                .supervisor
                .workspace()
                .process(PaneId::new(1))
                .unwrap()
                .state(),
            ProcessState::Pending
        );
    }

    /// A handled exit is acknowledged to its handle exactly once, so a holder
    /// lets go of it only after the supervisor processed it. A shutdown hands
    /// nothing back, leaving any exit to a later supervisor.
    #[test]
    fn a_handled_exit_is_acknowledged_and_a_shutdown_acknowledges_nothing() {
        let acks = Arc::new(AtomicUsize::new(0));
        let mut harness = started(
            Box::new(AckCountRunner {
                acks: Arc::clone(&acks),
            }),
            process(RestartPolicy::Never),
        );

        exit(&mut harness, ExitOutcome::Succeeded);
        assert_eq!(acks.load(Ordering::SeqCst), 1);

        harness.supervisor.start(&p(), &no_env());
        assert!(is_live(&harness));
        harness.supervisor.shutdown();
        assert_eq!(acks.load(Ordering::SeqCst), 1);
    }

    /// An exit the restart policy acts on stays unacknowledged until the
    /// respawn is installed, so a service stopping meanwhile leaves it for its
    /// successor; one whose restart a stop cancelled is acknowledged once that
    /// settles, so no holder is left waiting for a restart that never comes.
    #[test]
    fn a_restarting_exit_is_held_until_the_respawn_or_until_the_restart_settles() {
        let acks = Arc::new(AtomicUsize::new(0));
        let mut harness = started(
            Box::new(AckCountRunner {
                acks: Arc::clone(&acks),
            }),
            process(RestartPolicy::Always),
        );

        exit(&mut harness, ExitOutcome::FAILED_UNKNOWN);
        assert_eq!(state(&harness), ProcessState::Restarting);
        harness.supervisor.release_settled_exits();
        assert_eq!(acks.load(Ordering::SeqCst), 0, "held while restarting");
        harness
            .supervisor
            .handle_respawn(pane(), current_gen(&harness));
        assert!(is_live(&harness));
        assert_eq!(acks.load(Ordering::SeqCst), 1, "released by the respawn");

        exit(&mut harness, ExitOutcome::FAILED_UNKNOWN);
        harness.supervisor.stop(&p());
        assert_eq!(acks.load(Ordering::SeqCst), 1);
        harness.supervisor.release_settled_exits();
        assert_eq!(acks.load(Ordering::SeqCst), 2, "released once stopped");
    }

    /// A held exit a newer one displaces is acknowledged, never dropped, so
    /// its holder is not left waiting for an acknowledgement that never comes.
    #[test]
    fn a_held_exit_displaced_by_another_is_acknowledged() {
        let acks = Arc::new(AtomicUsize::new(0));
        let mut harness = started(
            Box::new(AckCountRunner {
                acks: Arc::clone(&acks),
            }),
            process(RestartPolicy::Always),
        );
        exit(&mut harness, ExitOutcome::FAILED_UNKNOWN);
        assert_eq!(acks.load(Ordering::SeqCst), 0, "held for the restart");

        harness.supervisor.install_pane(
            pane(),
            Box::new(AckCountHandle {
                acks: Arc::clone(&acks),
            }),
        );
        exit(&mut harness, ExitOutcome::FAILED_UNKNOWN);

        assert_eq!(acks.load(Ordering::SeqCst), 1, "the displaced exit");
        assert_eq!(harness.supervisor.held_exits.len(), 1);
    }

    /// `down` stops every running process and leaves stopped ones alone.
    #[test]
    fn down_stops_every_running_process() {
        let mut harness = Harness::over(Box::new(FakeRunner), &[
            spec("p", "true", RestartPolicy::Never),
            spec("q", "true", RestartPolicy::Never),
            spec("idle", "true", RestartPolicy::Never).with_autostart(false),
        ]);
        harness.up();

        let outcome = harness.supervisor.down();

        assert_eq!(outcome.acted(), &vec![p(), name("q")]);
        assert_eq!(state(&harness), ProcessState::Stopping);
        assert_eq!(
            *harness
                .supervisor
                .workspace()
                .process(PaneId::new(1))
                .unwrap()
                .state(),
            ProcessState::Stopping
        );
    }

    /// Failures on supervision paths become notices naming the process, and
    /// never panic: an unknown name, a failed spawn and an undeliverable stop.
    #[test]
    fn supervision_errors_land_in_notices_not_panics() {
        let mut harness = started(
            Box::new(FlakyRunner {
                spawns: AtomicUsize::new(0),
            }),
            process(RestartPolicy::Never),
        );

        harness.supervisor.start(&name("missing"), &no_env());
        harness.supervisor.stop(&p());
        exit(&mut harness, ExitOutcome::Succeeded);
        harness.supervisor.start(&p(), &no_env());

        let notices = harness
            .notices
            .lock()
            .unwrap()
            .recent(harness.supervisor.root());
        let messages: Vec<&str> = notices
            .iter()
            .map(|notice| notice.message().as_str())
            .collect();
        assert_eq!(messages[0], UNKNOWN_PROCESS_NOTICE);
        assert!(messages[1].starts_with(SPAWN_FAILED_NOTICE));
        assert_eq!(notices[1].process().as_ref(), Some(&p()));
        assert_eq!(*notices[1].level(), NoticeLevel::Error);
        assert_eq!(state(&harness), ProcessState::Crashed);
    }

    /// A handle whose holder answers the child's terminal queries gets no
    /// second reply from the supervisor; a bare PTY handle does.
    #[test]
    fn terminal_queries_are_answered_once() {
        /// A recording handle that may claim to answer queries itself.
        struct AnsweringHandle {
            written: Arc<Mutex<Vec<u8>>>,
            answers: bool,
        }

        impl ProcessHandle for AnsweringHandle {
            fn answers_terminal_queries(&self) -> bool {
                self.answers
            }

            fn write_input(&mut self, bytes: &[u8]) -> Result<(), PtyError> {
                self.written.lock().unwrap().extend_from_slice(bytes);
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

        /// Spawns [`AnsweringHandle`]s.
        struct AnsweringRunner {
            written: Arc<Mutex<Vec<u8>>>,
            answers: bool,
        }

        impl ProcessRunner for AnsweringRunner {
            fn spawn(
                &self,
                _request: SpawnRequest,
                _sink: Box<dyn OutputSink>,
            ) -> Result<Box<dyn ProcessHandle>, PtyError> {
                Ok(Box::new(AnsweringHandle {
                    written: self.written.clone(),
                    answers: self.answers,
                }))
            }
        }

        for answers in [true, false] {
            let written = Arc::new(Mutex::new(Vec::new()));
            let mut harness = started(
                Box::new(AnsweringRunner {
                    written: written.clone(),
                    answers,
                }),
                process(RestartPolicy::Never),
            );
            let generation = current_gen(&harness);

            harness.supervisor.handle_output(
                pane(),
                generation,
                ProcessOutput::Chunk(b"\x1b[6n".to_vec()),
            );

            assert_eq!(
                written.lock().unwrap().is_empty(),
                answers,
                "a cursor report is written only when the handle does not answer it"
            );
        }
    }

    /// A runner whose launches run on a worker and always fail.
    #[derive(Clone)]
    struct FailingWorkerRunner;

    impl ProcessRunner for FailingWorkerRunner {
        fn spawn(
            &self,
            request: SpawnRequest,
            sink: Box<dyn OutputSink>,
        ) -> Result<Box<dyn ProcessHandle>, PtyError> {
            FailingRunner.spawn(request, sink)
        }

        fn detached(&self) -> Option<Arc<dyn ProcessRunner + Send + Sync>> {
            Some(Arc::new(self.clone()))
        }
    }

    /// Output the launch delivers before its result reaches the loop.
    const EARLY_OUTPUT: &[u8] = b"then";

    /// A harness whose one process, `p` under `restart`, launches on a worker.
    fn on_worker(runner: Box<dyn ProcessRunner>, restart: RestartPolicy) -> Harness {
        Harness::new(runner, vec![process(restart)])
    }

    /// A launch on a worker leaves the process `Starting` until its result is
    /// settled; output that arrived first is replayed after the screen the
    /// connection began with, in order.
    #[test]
    fn a_worker_launch_installs_its_screen_then_the_output_that_came_first() {
        let mut harness = on_worker(Box::new(WorkerRunner::default()), RestartPolicy::Never);
        harness.supervisor.start(&p(), &no_env());
        assert_eq!(state(&harness), ProcessState::Starting);
        let (pane, generation, result) = spawned(&harness);

        harness.supervisor.handle_output(
            pane,
            generation,
            ProcessOutput::Chunk(EARLY_OUTPUT.to_vec()),
        );
        assert_eq!(state(&harness), ProcessState::Starting);
        harness.supervisor.handle_spawned(pane, generation, result);

        assert_eq!(state(&harness), ProcessState::Running);
        assert!(
            harness.supervisor.panes()[&pane]
                .parser()
                .screen_text()
                .contains("first then")
        );
    }

    /// A stop asked for while the launch was in flight reaches the process
    /// as soon as it is installed, through its stop policy.
    #[test]
    fn a_stop_while_starting_reaches_the_launched_process() {
        let mut harness = on_worker(Box::new(WorkerRunner::default()), RestartPolicy::Never);
        harness.supervisor.start(&p(), &no_env());
        harness.supervisor.stop(&p());
        assert_eq!(
            state(&harness),
            ProcessState::Starting,
            "kept for the launch"
        );
        let (pane, generation, result) = spawned(&harness);

        harness.supervisor.handle_spawned(pane, generation, result);

        assert_eq!(state(&harness), ProcessState::Stopping);
        assert_eq!(exit_intent(&harness), ExitIntent::StopInFlight);
    }

    /// A launch whose row was retired meanwhile is never installed: its
    /// process is killed when the result arrives.
    #[test]
    fn a_superseded_launch_is_killed_not_installed() {
        let runner = WorkerRunner::default();
        let kills = Arc::clone(&runner.0);
        let mut harness = on_worker(Box::new(runner), RestartPolicy::Never);
        harness.supervisor.start(&p(), &no_env());
        harness.supervisor.retire_pane(pane());
        let (pane, generation, result) = spawned(&harness);

        harness.supervisor.handle_spawned(pane, generation, result);

        assert_eq!(kills.load(Ordering::SeqCst), 1);
        assert!(!harness.supervisor.is_live(pane));
    }

    /// A launch still in flight when the service lets go of its processes is
    /// not cancelled: its result is detached, never killed, so its holder
    /// keeps the process for the next service, and it is no longer in flight.
    #[test]
    fn a_launch_in_flight_at_shutdown_is_detached_not_killed() {
        let runner = WorkerRunner::default();
        let kills = Arc::clone(&runner.0);
        let mut harness = on_worker(Box::new(runner), RestartPolicy::Never);
        harness.supervisor.start(&p(), &no_env());
        harness.supervisor.shutdown();
        assert_eq!(harness.supervisor.launches_in_flight(), 1);
        let (pane, generation, result) = spawned(&harness);

        harness.supervisor.handle_spawned(pane, generation, result);

        assert_eq!(kills.load(Ordering::SeqCst), 0, "never stopped");
        assert_eq!(harness.supervisor.launches_in_flight(), 0);
        assert!(!harness.supervisor.is_live(pane));
    }

    /// A launch a removal superseded is killed when its result arrives, even
    /// inside the window in which a shutdown detaches launches, whether the
    /// row retired before the shutdown or during it.
    #[test]
    fn a_launch_retired_around_shutdown_is_killed_not_detached() {
        for retire_first in [true, false] {
            let runner = WorkerRunner::default();
            let kills = Arc::clone(&runner.0);
            let mut harness = on_worker(Box::new(runner), RestartPolicy::Never);
            harness.supervisor.start(&p(), &no_env());
            if retire_first {
                harness.supervisor.retire_pane(pane());
                harness.supervisor.shutdown();
            } else {
                harness.supervisor.shutdown();
                harness.supervisor.retire_pane(pane());
            }
            let (pane, generation, result) = spawned(&harness);

            harness.supervisor.handle_spawned(pane, generation, result);

            assert_eq!(kills.load(Ordering::SeqCst), 1, "discarded, not detached");
            assert_eq!(harness.supervisor.launches_in_flight(), 0);
        }
    }

    /// A pause or resume asked for while starting is dropped when the launch
    /// installs nothing: only the failure is told.
    #[test]
    fn a_pause_while_starting_is_dropped_when_the_launch_fails() {
        for resume in [false, true] {
            let mut harness = on_worker(Box::new(FailingWorkerRunner), RestartPolicy::Never);
            harness.supervisor.start(&p(), &no_env());
            harness.supervisor.pause(&p());
            if resume {
                harness.supervisor.resume(&p());
            }
            let (pane, generation, result) = spawned(&harness);

            harness.supervisor.handle_spawned(pane, generation, result);

            assert_eq!(state(&harness), ProcessState::Crashed);
            let notices = harness.notice_messages();
            assert_eq!(notices.len(), 1, "only the failure is told: {notices:?}");
            assert!(notices[0].starts_with(SPAWN_FAILED_NOTICE));
        }
    }

    /// A pause asked for while starting reaches the process once installed,
    /// and a resume asked for after it cancels it.
    #[test]
    fn a_pause_or_resume_while_starting_applies_once_installed() {
        for (resume, expected) in [(false, ProcessState::Paused), (true, ProcessState::Running)] {
            let mut harness = on_worker(Box::new(WorkerRunner::default()), RestartPolicy::Never);
            harness.supervisor.start(&p(), &no_env());
            harness.supervisor.pause(&p());
            if resume {
                harness.supervisor.resume(&p());
            }
            assert_eq!(state(&harness), ProcessState::Starting);
            let (pane, generation, result) = spawned(&harness);

            harness.supervisor.handle_spawned(pane, generation, result);

            assert_eq!(state(&harness), expected);
        }
    }

    /// A process removed from the stack while it was starting takes that
    /// removal once installed: it is stopped, and its exit retires the row
    /// instead of restarting it.
    #[test]
    fn a_process_removed_while_starting_is_stopped_and_retires_on_exit() {
        let mut harness = on_worker(Box::new(WorkerRunner::default()), RestartPolicy::Always);
        harness.supervisor.start(&p(), &no_env());
        harness.supervisor.apply_stack(&stack(&[]));
        assert!(
            harness.supervisor.workspace().process(pane()).is_some(),
            "a starting row is kept until it exits"
        );
        let (pane, generation, result) = spawned(&harness);

        harness.supervisor.handle_spawned(pane, generation, result);
        assert_eq!(
            harness.supervisor.panes()[&pane].config_membership(),
            ConfigMembership::RetireOnExit
        );
        assert_eq!(state(&harness), ProcessState::Stopping);
        let current = harness.supervisor.generations()[&pane];
        harness.supervisor.handle_output(
            pane,
            current,
            ProcessOutput::Exited(ExitOutcome::FAILED_UNKNOWN),
        );

        assert!(harness.supervisor.workspace().process(pane).is_none());
    }

    /// The test process's spec, for a stack that keeps it.
    fn kept() -> ProcessSpec {
        spec("p", "true", RestartPolicy::Never)
    }

    /// A stack change that keeps a process leaves it as it runs.
    #[test]
    fn a_kept_process_is_untouched_by_a_stack_change() {
        let mut harness = running(RestartPolicy::Never);
        let generation = current_gen(&harness);

        harness.supervisor.apply_stack(&stack(&[kept()]));

        assert_eq!(state(&harness), ProcessState::Running);
        assert_eq!(current_gen(&harness), generation);
    }
}
