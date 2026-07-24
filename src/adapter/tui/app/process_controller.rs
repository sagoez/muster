use super::*;

/// Command details required to spawn one workspace pane.
type SpawnDetails = (Option<CommandLine>, Option<PathBuf>);

impl App {
    /// Resolves the exported project path and effective process directory.
    pub(super) fn resolve_spawn_paths(
        current_config: Option<&Path>,
        configured_working_dir: Option<PathBuf>,
    ) -> (Option<PathBuf>, Option<PathBuf>) {
        let project = current_config.map(path::absolutize);
        let workspace_dir = project
            .as_deref()
            .and_then(Path::parent)
            .map(Path::to_path_buf);
        let working_dir = match (workspace_dir, configured_working_dir) {
            (_, Some(directory)) if directory.is_absolute() => Some(directory),
            (Some(workspace), Some(directory)) => Some(workspace.join(directory)),
            (None, Some(directory)) => Some(directory),
            (Some(workspace), None) => Some(workspace),
            (None, None) => None,
        };
        (project, working_dir)
    }

    /// Spawns a process and connects its PTY to the runtime event stream.
    pub(super) fn spawn(
        &mut self,
        pane: PaneId,
        command: Option<CommandLine>,
        cwd: Option<PathBuf>,
    ) {
        let generation = self.bump_generation(pane);
        let activity = self
            .workspace
            .process(pane)
            .map(ActivityTracker::for_process)
            .unwrap_or_default();
        let (project, cwd) = Self::resolve_spawn_paths(self.current_config.as_deref(), cwd);
        let agent_session_id = self
            .workspace
            .process(pane)
            .and_then(|process| process.agent_session_id().as_ref())
            .cloned();
        let environment = agent_session_id
            .as_ref()
            .map_or_else(BTreeMap::new, |session_id| {
                let mut environment = BTreeMap::from([(
                    OsString::from(MUSTER_AGENT_SESSION_ENV),
                    OsString::from(session_id.as_ref()),
                )]);
                if let Some(store) = &self.agent_session_store {
                    match store.state_file_path() {
                        Ok(Some(path)) => {
                            environment.insert(
                                OsString::from(MUSTER_AGENT_SESSION_STATE_FILE_ENV),
                                path.into_os_string(),
                            );
                        },
                        Ok(None) => {},
                        Err(error) => {
                            self.notice = Some(format!("{AGENT_SESSION_STORE_ERROR}: {error}"))
                        },
                    }
                }
                environment
            });
        let request = SpawnRequest::builder()
            .command(command)
            .working_dir(cwd)
            .project(project)
            .environment(environment)
            .agent_session_id(agent_session_id.clone())
            .size(self.pane_size)
            .build();
        let sink = ChannelOutputSink::new(pane, generation, self.events.clone());
        match self.runner.spawn(request, Box::new(sink)) {
            Ok(mut handle) => {
                if let Some(session_id) = &agent_session_id
                    && let Some(store) = &self.agent_session_store
                    && let Err(error) = store.set_state(session_id, AgentSessionState::Open)
                {
                    let _ = handle.kill();
                    self.deactivate(pane);
                    self.workspace.set_state(pane, ProcessState::Crashed);
                    self.notice = Some(format!("{AGENT_SESSION_STORE_ERROR}: {error}"));
                    return;
                }
                let notification_scope = self.allocate_notification_scope();
                let parser = Parser::new(
                    self.pane_size.rows().into_inner(),
                    self.pane_size.cols().into_inner(),
                    SCROLLBACK_LINES,
                );
                self.panes.insert(pane, Pane {
                    parser,
                    notification_scope,
                    signals: SignalReader::new(),
                    activity,
                    last_bell: None,
                    pending_bell_notification: None,
                    handle: Some(handle),
                    started_at: Instant::now(),
                    exit_intent: ExitIntent::FollowPolicy,
                    shutdown_generation: ShutdownGeneration::initial(),
                    config_membership: ConfigMembership::Tracked,
                });
                self.workspace.set_state(pane, ProcessState::Running);
                self.workspace.set_activity(pane, ActivityState::Idle);
            },
            Err(_) => {
                self.deactivate(pane);
                if self.workspace.should_restart(pane, ExitOutcome::Failed) {
                    self.workspace.set_state(pane, ProcessState::Restarting);
                    self.schedule_restart(pane, Instant::now());
                } else {
                    self.workspace.set_state(pane, ProcessState::Crashed);
                }
            },
        }
    }

    /// Invalidates pending respawns and returns the new generation.
    pub(super) fn bump_generation(&mut self, pane: PaneId) -> SpawnGeneration {
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

    /// Allocates a notification scope unique to this terminal lifetime.
    pub(super) fn allocate_notification_scope(&mut self) -> NotificationScope {
        let scope = self.next_notification_scope;
        self.next_notification_scope = scope.next();
        scope
    }

    /// Drops a live handle while retaining the final terminal screen.
    pub(super) fn deactivate(&mut self, pane: PaneId) {
        if let Some(target) = self.panes.get_mut(&pane) {
            target.activity.reset();
            // A bell only matters while its child can still need attention.
            target.pending_bell_notification = None;
            target.handle = None;
            target.exit_intent = ExitIntent::FollowPolicy;
        }
    }

    /// Retires a process that no longer belongs in the workspace.
    pub(super) fn retire_pane(&mut self, pane: PaneId) {
        self.panes.remove(&pane);
        self.generations.remove(&pane);
        self.restart_attempts.remove(&pane);
        self.workspace.remove(pane);
    }

    /// Applies the domain lifecycle decision to one PTY exit event.
    pub(super) fn handle_exit(&mut self, pane: PaneId, outcome: ExitOutcome) {
        let Some((exit_intent, config_membership, started_at)) =
            self.panes.get(&pane).map(|target| {
                (
                    target.exit_intent,
                    target.config_membership,
                    target.started_at,
                )
            })
        else {
            return;
        };
        let decision = ProcessLifecycle::after_exit(
            exit_intent,
            config_membership == ConfigMembership::RetireOnExit,
            self.workspace.should_restart(pane, outcome),
            outcome,
        );
        if decision == ExitDecision::Retire {
            let reopen = self.pending_session_reopens.remove(&pane);
            self.retire_pane(pane);
            self.advance_pending_switch(pane);
            if let Some(session_id) = reopen {
                self.reopen_agent_session(&session_id);
            }
            return;
        }
        self.workspace.set_activity(pane, ActivityState::Idle);
        match decision {
            ExitDecision::Stop => {
                self.deactivate(pane);
                self.workspace.set_state(pane, ProcessState::Exited);
            },
            ExitDecision::RestartNow => {
                self.restart_attempts.remove(&pane);
                self.workspace.set_state(pane, ProcessState::Restarting);
                match self.command_of(pane) {
                    Ok(Some((command, cwd))) => self.spawn(pane, command, cwd),
                    Ok(None) => self.settle_unresumable_restart(pane),
                    Err(error) => self.settle_restart_store_failure(pane, error),
                }
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
            ExitDecision::Retire => unreachable!(),
        }
        self.advance_pending_switch(pane);
    }

    /// Schedules a backoff restart and tags it with the current generation.
    pub(super) fn schedule_restart(&mut self, pane: PaneId, started_at: Instant) {
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
        thread::spawn(move || {
            thread::sleep(delay);
            let _ = sender.send(RuntimeEvent::Respawn { pane, generation });
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
        thread::spawn(move || {
            thread::sleep(grace_period);
            let _ = sender.send(RuntimeEvent::ForceStop {
                pane,
                spawn_generation,
                shutdown_generation,
            });
        });
    }

    /// Returns the command and working directory for a pane's next spawn.
    pub(super) fn command_of(&self, pane: PaneId) -> Result<Option<SpawnDetails>, ConfigError> {
        let Some(process) = self.workspace.process(pane) else {
            return Ok(None);
        };
        let command = if let Some(session_id) = process.agent_session_id() {
            let Some(session) = self
                .agent_sessions()?
                .into_iter()
                .find(|session| session.id() == session_id)
            else {
                return Ok(None);
            };
            let Some(command) = session.restore_command() else {
                return Ok(None);
            };
            Some(command)
        } else {
            process.command().clone()
        };
        Ok(Some((command, process.working_dir().clone())))
    }

    /// Reports a session-state read failure without confusing it with an absent identity.
    fn report_session_store_error(&mut self, error: ConfigError) {
        self.notice = Some(format!("{AGENT_SESSION_STORE_ERROR}: {error}"));
    }

    /// Settles a failed restart whose durable session has no safe launch command.
    fn settle_unresumable_restart(&mut self, pane: PaneId) {
        self.deactivate(pane);
        self.workspace.set_state(pane, ProcessState::Crashed);
        self.notice = Some(AGENT_SESSION_NOT_RESUMABLE.to_string());
    }

    /// Settles a failed restart after preserving the durable-store error for the user.
    fn settle_restart_store_failure(&mut self, pane: PaneId, error: ConfigError) {
        self.deactivate(pane);
        self.workspace.set_state(pane, ProcessState::Crashed);
        self.report_session_store_error(error);
    }

    /// Toggles the selected process: stop it if alive, start it if not.
    pub(super) fn toggle_selected(&mut self) {
        let Some(pane) = self.selected_pane() else {
            return;
        };
        if self
            .panes
            .get(&pane)
            .is_some_and(|target| target.handle.is_some())
        {
            self.stop_selected();
        } else {
            self.start_selected();
        }
    }

    /// Starts the selected process if it is not currently running.
    pub(super) fn start_selected(&mut self) {
        let Some(pane) = self.selected_pane() else {
            return;
        };
        match self.command_of(pane) {
            Ok(Some((command, cwd))) => self.spawn(pane, command, cwd),
            Ok(None) => self.notice = Some(AGENT_SESSION_NOT_RESUMABLE.to_string()),
            Err(error) => self.report_session_store_error(error),
        }
    }

    /// Toggles the selected child between paused and running through its PTY handle.
    pub(super) fn toggle_pause_selected(&mut self) {
        let Some(pane) = self.selected_pane() else {
            return;
        };
        let is_paused = self
            .workspace
            .process(pane)
            .is_some_and(|process| *process.state() == ProcessState::Paused);
        let now_paused = {
            let Some(target) = self.panes.get_mut(&pane) else {
                return;
            };
            if target.exit_intent != ExitIntent::FollowPolicy {
                return;
            }
            let Some(handle) = target.handle.as_mut() else {
                return;
            };
            let next = !is_paused;
            let signalled = if next {
                handle.pause()
            } else {
                handle.resume()
            };
            if signalled.is_err() {
                return;
            }
            next
        };
        self.workspace.set_state(
            pane,
            if now_paused {
                ProcessState::Paused
            } else {
                ProcessState::Running
            },
        );
    }

    /// Restarts the selected process regardless of its configured restart policy.
    pub(super) fn restart_selected(&mut self) {
        let Some(pane) = self.selected_pane() else {
            return;
        };
        let command = match self.command_of(pane) {
            Ok(Some(command)) => command,
            Ok(None) => {
                self.notice = Some(AGENT_SESSION_NOT_RESUMABLE.to_string());
                return;
            },
            Err(error) => {
                self.report_session_store_error(error);
                return;
            },
        };
        if self
            .panes
            .get(&pane)
            .is_some_and(|target| target.handle.is_some())
        {
            if self
                .panes
                .get(&pane)
                .is_some_and(|target| !target.exit_intent.accepts_restart_request())
            {
                return;
            }
            self.request_graceful_transition(pane, ExitIntent::request_restart, true);
        } else {
            let (command, cwd) = command;
            self.spawn(pane, command, cwd);
        }
    }

    /// Stops the selected process without allowing its restart policy to respawn it.
    pub(super) fn stop_selected(&mut self) {
        let Some(pane) = self.selected_pane() else {
            return;
        };
        if self.panes.get(&pane).is_some_and(|target| {
            target.handle.is_some() && !target.exit_intent.accepts_stop_request()
        }) {
            return;
        }
        if self
            .panes
            .get(&pane)
            .is_some_and(|target| target.handle.is_some())
        {
            self.request_graceful_transition(pane, ExitIntent::request_stop, false);
        } else {
            self.bump_generation(pane);
            self.deactivate(pane);
            self.workspace.set_state(pane, ProcessState::Exited);
        }
    }

    /// Sends the configured graceful signal and schedules a single matching escalation.
    fn request_graceful_transition(
        &mut self,
        pane: PaneId,
        request: impl FnOnce(ExitIntent) -> ExitIntent,
        restart: bool,
    ) {
        let policy = self.stop_policy_of(pane);
        let spawn_generation = self.generations.get(&pane).copied();
        let shutdown_generation = self.advance_shutdown_generation(pane);
        let mut awaiting_grace = None;
        let mut used_fallback = false;
        let mut delivered = false;
        if let Some(target) = self.panes.get_mut(&pane) {
            target.exit_intent = request(target.exit_intent);
            if let Some(handle) = target.handle.as_mut() {
                delivered = if let Some(policy) = &policy {
                    match handle.terminate(*policy.signal(), *policy.grace_period()) {
                        Ok(()) => {
                            awaiting_grace = Some(*policy.grace_period());
                            true
                        },
                        Err(_) => {
                            used_fallback = true;
                            handle.kill().is_ok()
                        },
                    }
                } else {
                    handle.kill().is_ok()
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
                self.notice = Some(GRACEFUL_STOP_FALLBACK_NOTICE.to_string());
            }
        } else {
            self.notice = Some(STOP_DELIVERY_FAILED_NOTICE.to_string());
        }
        if let (Some(grace_period), Some(spawn_generation), Some(shutdown_generation)) =
            (awaiting_grace, spawn_generation, shutdown_generation)
        {
            self.schedule_force_stop(pane, spawn_generation, shutdown_generation, grace_period);
        }
    }

    /// Immediately force-kills the selected process and cancels a pending respawn.
    pub(super) fn force_stop_selected(&mut self) {
        let Some(pane) = self.selected_pane() else {
            return;
        };
        if self
            .panes
            .get(&pane)
            .is_some_and(|target| target.handle.is_some())
        {
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
                self.notice = Some(STOP_DELIVERY_FAILED_NOTICE.to_string());
            }
        } else {
            self.bump_generation(pane);
            self.deactivate(pane);
            self.workspace.set_state(pane, ProcessState::Exited);
        }
    }

    /// Returns the selected command's configured or default shutdown policy.
    fn stop_policy_of(&self, pane: PaneId) -> Option<StopPolicy> {
        self.workspace
            .process(pane)
            .and_then(Process::effective_stop_policy)
    }

    /// Returns the pane selected in the active workspace.
    pub(super) fn selected_pane(&self) -> Option<PaneId> {
        self.workspace
            .selected_process()
            .map(|process| *process.id())
    }

    /// Applies one process output event unless a newer spawn superseded it.
    pub(crate) fn handle_output(
        &mut self,
        pane: PaneId,
        generation: SpawnGeneration,
        output: ProcessOutput,
    ) {
        if self.generations.get(&pane) != Some(&generation) {
            return;
        }
        match output {
            ProcessOutput::Chunk(bytes) => {
                let Some(target) = self.panes.get_mut(&pane) else {
                    return;
                };
                let signals = target.signals.read(&bytes);
                target.parser.process(&bytes);
                for signal in signals {
                    self.apply_signal(pane, signal);
                }
            },
            ProcessOutput::Exited(outcome) => self.handle_exit(pane, outcome),
        }
    }

    /// Applies one decoded terminal lifecycle signal to a managed pane.
    fn apply_signal(&mut self, pane: PaneId, signal: Signal) {
        match signal {
            Signal::Output => {
                let now = Instant::now();
                let activity = self.panes.get_mut(&pane).and_then(|target| {
                    if let Some(deadline) = target.pending_bell_notification.as_mut() {
                        *deadline = now + BELL_ESCALATION_DELAY;
                        None
                    } else {
                        target.activity.observe_output(now)
                    }
                });
                if let Some(activity) = activity {
                    self.workspace.set_activity(pane, activity);
                }
            },
            Signal::Title(title) => {
                let activity = self
                    .panes
                    .get_mut(&pane)
                    .and_then(|target| target.activity.observe_title(Instant::now(), title));
                if let Some(activity) = activity {
                    self.workspace.set_activity(pane, activity);
                }
            },
            Signal::Progress(active) => {
                let activity = self
                    .panes
                    .get_mut(&pane)
                    .map(|target| target.activity.observe_progress(active));
                if let Some(activity) = activity {
                    self.workspace.set_activity(pane, activity);
                }
            },
            Signal::Notify {
                identifier,
                title,
                body,
            } => {
                if let Some(target) = self.panes.get_mut(&pane) {
                    self.workspace
                        .set_activity(pane, target.activity.observe_attention());
                }
                if title.is_none() && body.is_none() && !self.accept_bell(pane) {
                    return;
                }
                if title.is_none() && body.is_none() && self.defer_or_suppress_non_agent_bell(pane)
                {
                    return;
                }
                self.raise_notification(pane, identifier, title, body);
            },
            Signal::Close { identifier } => self.close_notification(pane, &identifier),
        }
    }

    /// Returns the nearest time at which ordinary output should become idle.
    pub(crate) fn next_activity_deadline(&self) -> Option<Instant> {
        self.panes
            .values()
            .flat_map(|pane| [pane.activity.deadline(), pane.pending_bell_notification])
            .flatten()
            .min()
    }

    /// Expires quiet-output activity deadlines and reports whether any pane changed.
    pub(crate) fn expire_quiet_activity(&mut self, now: Instant) -> bool {
        let expired = self
            .panes
            .iter_mut()
            .filter_map(|(pane, target)| {
                target
                    .activity
                    .expire(now)
                    .map(|activity| (*pane, activity))
            })
            .collect::<Vec<_>>();
        for (pane, activity) in &expired {
            self.workspace.set_activity(*pane, *activity);
        }
        let due_bells = self
            .panes
            .iter_mut()
            .filter_map(|(pane, target)| {
                target
                    .pending_bell_notification
                    .is_some_and(|deadline| deadline <= now)
                    .then_some(*pane)
            })
            .collect::<Vec<_>>();
        for pane in &due_bells {
            if let Some(target) = self.panes.get_mut(pane) {
                target.pending_bell_notification = None;
            }
            if self.selected_pane() != Some(*pane) {
                self.raise_notification(*pane, None, None, None);
            }
        }
        !expired.is_empty() || !due_bells.is_empty()
    }

    /// Returns the next spinner frame deadline while a running agent is working.
    pub(crate) fn next_activity_frame_deadline(&self) -> Option<Instant> {
        self.workspace
            .processes()
            .iter()
            .any(|process| {
                *process.kind() == ProcessKind::Agent
                    && *process.state() == ProcessState::Running
                    && *process.activity() == ActivityState::Working
            })
            .then_some(self.activity_frame_deadline)
    }

    /// Advances the working-agent spinner when its next frame is due.
    pub(crate) fn advance_activity_frame(&mut self, now: Instant) -> bool {
        if self
            .next_activity_frame_deadline()
            .is_none_or(|deadline| deadline > now)
        {
            return false;
        }
        self.activity_frame = self.activity_frame.next();
        self.activity_frame_deadline = now + ACTIVITY_FRAME_INTERVAL;
        true
    }

    /// Raises a scoped in-app and optional desktop notification from one pane.
    fn raise_notification(
        &mut self,
        pane: PaneId,
        identifier: Option<NotificationId>,
        title: Option<String>,
        body: Option<String>,
    ) {
        let is_bell = title.is_none() && body.is_none();
        let Some(scope) = self
            .panes
            .get(&pane)
            .map(|target| target.notification_scope)
        else {
            return;
        };
        let Some(process) = self.workspace.process(pane) else {
            return;
        };
        let name = process.name().clone();
        let desktop_body = is_bell
            .then(|| AWAITING_INPUT_NOTICE.to_string())
            .or(body.clone());
        let message = body
            .or_else(|| title.clone())
            .unwrap_or_else(|| AWAITING_INPUT_NOTICE.to_string());
        self.notice = Some(format!("{}: {message}", name.as_ref()));
        let notification = Notification::builder()
            .pane(pane)
            .scope(scope)
            .source(name)
            .title(title)
            .body(desktop_body)
            .identifier(identifier)
            .build();
        if self.desktop_notifications_enabled()
            && let Some(notifier) = &self.notifier
        {
            notifier.notify(&notification);
        }
    }

    /// Accepts a bare bell unless another bell from this pane arrived too recently.
    fn accept_bell(&mut self, pane: PaneId) -> bool {
        let Some(target) = self.panes.get_mut(&pane) else {
            return false;
        };
        let now = Instant::now();
        if target
            .last_bell
            .is_some_and(|last| now.duration_since(last) < BELL_THROTTLE)
        {
            return false;
        }
        target.last_bell = Some(now);
        true
    }

    /// Suppresses a foreground non-agent bell or delays its background desktop alert.
    fn defer_or_suppress_non_agent_bell(&mut self, pane: PaneId) -> bool {
        let Some(process) = self.workspace.process(pane) else {
            return false;
        };
        if *process.kind() == ProcessKind::Agent {
            return false;
        }
        if self.selected_pane() != Some(pane)
            && let Some(target) = self.panes.get_mut(&pane)
        {
            target.pending_bell_notification = Some(Instant::now() + BELL_ESCALATION_DELAY);
        }
        true
    }

    /// Closes one prior identified desktop notification for its pane lifetime.
    fn close_notification(&self, pane: PaneId, identifier: &NotificationId) {
        if let Some(scope) = self
            .panes
            .get(&pane)
            .map(|target| target.notification_scope)
            && let Some(notifier) = &self.notifier
        {
            notifier.close(pane, scope, identifier);
        }
    }

    /// Respawns a pane only when its delayed restart still belongs to this generation.
    pub(crate) fn handle_respawn(&mut self, pane: PaneId, generation: SpawnGeneration) {
        if self.generations.get(&pane) != Some(&generation) {
            return;
        }
        match self.command_of(pane) {
            Ok(Some((command, cwd))) => self.spawn(pane, command, cwd),
            Ok(None) => self.settle_unresumable_restart(pane),
            Err(error) => self.settle_restart_store_failure(pane, error),
        }
    }

    /// Escalates an elapsed graceful stop only when no newer lifecycle action superseded it.
    pub(crate) fn handle_force_stop(
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
        let Some(handle) = target.handle.as_mut() else {
            return;
        };
        if handle.kill().is_ok() {
            self.workspace.set_state(pane, ProcessState::Stopping);
            self.notice = Some(FORCE_STOP_NOTICE.to_string());
        } else {
            target.exit_intent = target.exit_intent.force_stop_delivery_failed();
            self.notice = Some(STOP_DELIVERY_FAILED_NOTICE.to_string());
        }
    }
}
