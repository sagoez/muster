use super::*;

impl App {
    /// Handles a key while a form is open: edit, submit, or cancel.
    pub(super) fn handle_form_key(&mut self, key: KeyEvent) {
        let Some(modal) = self.form_mut() else {
            return;
        };
        let before = (modal.form.active(), modal.form.active_path_value());
        let outcome = modal.form.handle(key);
        // Recompute completions only when the active field or its value changed,
        // so dropdown navigation (which only moves the highlight) keeps it.
        let changed = (modal.form.active(), modal.form.active_path_value()) != before;
        match outcome {
            FormOutcome::Continue => {
                if changed {
                    self.refresh_completions();
                }
            },
            // Acceptance closes the dropdown deliberately; leave it closed so the
            // next Enter submits instead of accepting a child of the chosen dir.
            FormOutcome::Accepted => {},
            FormOutcome::Cancel => self.close_overlay(),
            FormOutcome::Submit => self.submit_form(),
        }
    }

    /// Recomputes the active path field's autocomplete candidates. With a worker
    /// wired, it dispatches a generation-tagged request and clears the field so
    /// no stale suggestion is shown until the matching reply arrives; otherwise
    /// it completes inline.
    pub(super) fn refresh_completions(&mut self) {
        let Some(partial) = self.form().and_then(|modal| modal.form.active_path_value()) else {
            return;
        };
        // The candidates to show right now: none while an async request is in
        // flight (its reply repopulates them, and until then no navigation or
        // acceptance can act on suggestions that no longer match the edited
        // value), or the inline result otherwise.
        let candidates = match &mut self.completion_mode {
            CompletionMode::Worker {
                requests,
                generation,
            } => {
                *generation = generation.next();
                let _ = requests.send(CompletionRequest {
                    generation: *generation,
                    partial,
                });
                Vec::new()
            },
            CompletionMode::Inline(completer) => completer.complete_dir(&partial),
        };
        if let Some(modal) = self.form_mut() {
            modal.form.set_active_candidates(candidates);
        }
    }

    /// Applies worker-computed completions, ignoring any that a later edit has
    /// already superseded.
    pub fn handle_completions(
        &mut self,
        generation: CompletionGeneration,
        candidates: Vec<String>,
    ) {
        let CompletionMode::Worker {
            generation: current,
            ..
        } = &self.completion_mode
        else {
            return;
        };
        if generation != *current {
            return;
        }
        if let Some(modal) = self.form_mut() {
            modal.form.set_active_candidates(candidates);
        }
    }

    /// Executes the open form's intent with its collected values.
    pub(super) fn submit_form(&mut self) {
        let Some(modal) = self.form() else {
            return;
        };
        let values = modal.form.values();
        let intent = modal.intent;
        match intent {
            FormIntent::SaveCurrentProject => self.save_current_project(&values),
            FormIntent::NewProject => self.new_project(&values),
            FormIntent::ChooseProcessKind => self.choose_process_kind(&values),
            FormIntent::LaunchAgentSession => self.launch_agent_session(&values),
            FormIntent::AddConfiguredProcess(kind) => {
                self.add_configured_process(kind, &values);
            },
        }
    }

    /// Registers the current workspace under the typed name. A blank or invalid
    /// name leaves the form open.
    pub(super) fn save_current_project(&mut self, values: &[String]) {
        let (Some(name), Some(config)) = (values.first(), self.current_config.clone()) else {
            return;
        };
        let Ok(name) = ProjectName::try_new(name.trim()) else {
            return;
        };
        if self.try_register(Project::builder().name(name).config(config).build()) {
            self.close_overlay();
            self.refresh_projects();
            self.refresh_switcher();
        }
    }

    /// Creates a new project, asking first if the folder already holds a config.
    /// A blank or invalid field leaves the form open.
    pub(super) fn new_project(&mut self, values: &[String]) {
        let (Some(name), Some(folder)) = (values.first(), values.get(1)) else {
            return;
        };
        let Ok(name) = ProjectName::try_new(name.trim()) else {
            return;
        };
        let folder = folder.trim();
        if folder.is_empty() {
            return;
        }
        let config_path = PathBuf::from(folder).join(WORKSPACE_FILE_NAME);
        if self.registry.workspace_exists(&config_path) {
            self.confirm_overwrite(name, config_path);
            return;
        }
        self.create_project(name, config_path);
    }

    /// Opens a confirmation over the form before overwriting an existing config.
    /// The form is kept so a failed overwrite can be retried without refilling.
    pub(super) fn confirm_overwrite(&mut self, name: ProjectName, config_path: PathBuf) {
        let Some(Overlay::Form(form)) = self.overlay.take() else {
            return;
        };
        self.overlay = Some(Overlay::ConfirmOverwrite {
            form,
            name,
            config_path,
        });
    }

    /// Registers the project, then writes its starter config. Registration comes
    /// first so a failed write leaves a recoverable dangling entry (a retry heals
    /// it) rather than a stranded file that would block re-creation.
    pub(super) fn create_project(&mut self, name: ProjectName, config_path: PathBuf) {
        let project = Project::builder()
            .name(name)
            .config(config_path.clone())
            .build();
        if !self.try_register(project) {
            return;
        }
        if self
            .registry
            .save_workspace(&config_path, &starter_workspace())
            .is_err()
        {
            self.report_error(WORKSPACE_SAVE_ERROR);
            return;
        }
        self.close_overlay();
        self.refresh_projects();
        self.refresh_switcher();
    }

    /// Handles a key while a confirmation is open: accept (y / Enter) or cancel.
    pub(super) fn handle_confirm_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Enter | KeyCode::Char('y') => {
                let overlay = self.overlay.take();
                match overlay {
                    Some(Overlay::ConfirmOverwrite {
                        form,
                        name,
                        config_path,
                    }) => {
                        self.overlay = Some(Overlay::Form(form));
                        self.create_project(name, config_path);
                    },
                    Some(Overlay::ConfirmRemoval { config_path, .. }) => {
                        self.remove_project(&config_path);
                    },
                    Some(Overlay::ConfirmSessionClose { pane, .. }) => {
                        self.close_agent_session(pane);
                    },
                    other => self.overlay = other,
                }
            },
            KeyCode::Esc | KeyCode::Char('n') => {
                self.close_overlay();
            },
            _ => {},
        }
    }

    /// Reports an error in the active modal, falling back to a status notice.
    pub(super) fn report_error(&mut self, message: &str) {
        if let Some(modal) = self.form_mut() {
            modal.error = Some(message.to_string());
        } else if let Some(switcher) = self.switcher_mut() {
            switcher.error = Some(message.to_string());
        } else if let Some(Overlay::AgentPicker(picker)) = &mut self.overlay {
            picker.error = Some(message.to_string());
        } else {
            self.set_notice(message.to_string());
        }
    }

    /// Adds `project` to the registry, replacing any entry for the same config
    /// path, atomically under the registry lock. Returns whether it was
    /// persisted, setting a form error on failure; leaves the form open so the
    /// caller decides when to close it.
    pub(super) fn try_register(&mut self, project: Project) -> bool {
        let project = Project::builder()
            .name(project.name().clone())
            .config(path::absolutize(project.config()))
            .build();
        let result = self.registry.update_projects(&mut |mut projects| {
            projects.retain(|existing| {
                !Self::same_config_location(existing.config(), project.config())
            });
            projects.push(project.clone());
            projects
        });
        match result {
            Ok(()) => true,
            Err(error) => {
                self.report_error(&error.to_string());
                false
            },
        }
    }

    /// Advances the add flow to the selected kind's specific form. Every kind,
    /// agents included, opens the configured-process form so the new process is
    /// pinned in `muster.yml` and its autostart is controllable with `t`. The
    /// disposable-session picker is a separate flow reached with `A`.
    pub(super) fn choose_process_kind(&mut self, values: &[String]) {
        match values.first().map(String::as_str) {
            Some(KIND_AGENT) => self.open_configured_process_form(ProcessKind::Agent),
            Some(KIND_TERMINAL) => self.open_configured_process_form(ProcessKind::Terminal),
            Some(KIND_COMMAND) => self.open_configured_process_form(ProcessKind::Command),
            _ => {},
        }
    }

    /// Launches a durable coding-agent session without modifying `muster.yml`.
    pub(super) fn launch_agent_session(&mut self, values: &[String]) {
        let (Some(tool), Some(name), Some(command)) =
            (values.first(), values.get(1), values.get(2))
        else {
            return;
        };
        let Ok(tool) = tool.parse::<AgentTool>() else {
            return;
        };
        let name = if name.trim().is_empty() {
            None
        } else {
            ProcessName::try_new(name).ok()
        };
        let command = if command.trim().is_empty() {
            None
        } else {
            CommandLine::try_new(command).ok()
        };
        let resume = values.get(3).and_then(|resume| {
            (!resume.trim().is_empty())
                .then(|| CommandLine::try_new(resume))
                .and_then(Result::ok)
        });
        self.create_agent_session(tool, name, command, resume);
    }

    /// Creates, persists, launches, and attaches a new agent conversation.
    pub(super) fn create_agent_session(
        &mut self,
        tool: AgentTool,
        name: Option<ProcessName>,
        command: Option<CommandLine>,
        resume_command: Option<CommandLine>,
    ) {
        if resume_command
            .as_ref()
            .is_some_and(|template| !AgentSession::resume_template_is_valid(template))
        {
            self.report_error(AGENT_RESUME_TEMPLATE_INVALID);
            return;
        }
        let Some(project) = self.current_config.clone() else {
            return;
        };
        let command = match command.or_else(|| {
            tool.default_command()
                .and_then(|command| CommandLine::try_new(command).ok())
        }) {
            Some(command) => command,
            None => {
                self.report_error(AGENT_COMMAND_REQUIRED);
                return;
            },
        };
        let Ok(id) = AgentSessionId::generate() else {
            self.report_error(AGENT_SESSION_STORE_ERROR);
            return;
        };
        let name = match name.or_else(|| self.generated_agent_name(tool, &id)) {
            Some(name) => name,
            None => {
                self.report_error(AGENT_SESSION_STORE_ERROR);
                return;
            },
        };
        let Some(launch_command) = tool.new_session_command(&command, &id) else {
            self.report_error(AGENT_COMMAND_REQUIRED);
            return;
        };
        let session = AgentSession::builder()
            .id(id.clone())
            .name(name)
            .tool(tool)
            .project(project)
            .launch_command(command.clone())
            .resume_command(resume_command)
            .state(AgentSessionState::Pending)
            .build();
        if let Some(store) = &self.agent_session_store
            && let Err(error) = store.upsert(&session)
        {
            self.report_error(&format!("{AGENT_SESSION_STORE_ERROR}: {error}"));
            return;
        }
        self.insert_agent_session(
            &session,
            AgentSessionActivation::StartAttached(launch_command),
        );
    }

    /// Generates a friendly non-identifying name, avoiding active-row
    /// collisions when possible and falling back to the provider plus UUID.
    pub(super) fn generated_agent_name(
        &self,
        tool: AgentTool,
        id: &AgentSessionId,
    ) -> Option<ProcessName> {
        for _ in 0..GENERATED_NAME_ATTEMPTS {
            let generated: String = FirstName().fake();
            if let Ok(name) = ProcessName::try_new(generated)
                && !self.name_in_use(&name)
            {
                return Some(name);
            }
        }
        let suffix: String = id.as_ref().chars().take(8).collect();
        ProcessName::try_new(format!("{tool} {suffix}")).ok()
    }

    /// Resolves the name for a newly added configured process. An agent's name is
    /// optional and must be unique among the project's agents (it is the durable
    /// session key): a blank input autogenerates a fresh name, while a provided
    /// one is rejected if it collides. Terminals and commands require a name and
    /// silently reject a blank or invalid one, matching the form's own validation.
    fn resolve_configured_name(&mut self, kind: ProcessKind, input: &str) -> Option<ProcessName> {
        if kind == ProcessKind::Agent && input.is_empty() {
            return self.generate_configured_agent_name();
        }
        let name = ProcessName::try_new(input).ok()?;
        if kind == ProcessKind::Agent && self.configured_agent_name_taken(&name) {
            self.report_error(AGENT_NAME_TAKEN);
            return None;
        }
        Some(name)
    }

    /// Whether a configured agent already uses `name`, which would collide the
    /// durable session key. Only agents are checked; sharing a name with a
    /// terminal or command is allowed, since those are not session-keyed.
    fn configured_agent_name_taken(&self, name: &ProcessName) -> bool {
        self.workspace.processes().iter().any(|process| {
            *process.kind() == ProcessKind::Agent
                && *process.origin() == ProcessOrigin::Configured
                && process.name() == name
        })
    }

    /// Generates a friendly agent name unused by any current process, so a
    /// UI-created agent's project-unique session key is guaranteed. Falls back to
    /// a numbered name bounded by the process count, which always finds a free slot.
    fn generate_configured_agent_name(&self) -> Option<ProcessName> {
        for _ in 0..GENERATED_NAME_ATTEMPTS {
            let generated: String = FirstName().fake();
            if let Ok(name) = ProcessName::try_new(generated)
                && !self.name_in_use(&name)
            {
                return Some(name);
            }
        }
        let ceiling = self.workspace.processes().len() + 1;
        (1..=ceiling).find_map(|suffix| {
            ProcessName::try_new(format!("{GENERATED_AGENT_PREFIX} {suffix}"))
                .ok()
                .filter(|name| !self.name_in_use(name))
        })
    }

    /// Whether any current process, of any kind, already uses `name`.
    fn name_in_use(&self, name: &ProcessName) -> bool {
        self.workspace
            .processes()
            .iter()
            .any(|process| process.name() == name)
    }

    /// Inserts one persisted session as a process, applying its requested
    /// stopped, detached-start, or attached-start activation.
    pub(super) fn insert_agent_session(
        &mut self,
        session: &AgentSession,
        activation: AgentSessionActivation,
    ) {
        if let Some(process) = self
            .workspace
            .processes()
            .iter()
            .find(|process| process.agent_session_id().as_ref() == Some(session.id()))
        {
            let pane = *process.id();
            if let Some(index) = self.workspace.position_of(pane) {
                self.workspace.select_at(index);
            }
            if let Some(command) = activation.command().cloned()
                && self
                    .panes
                    .get(&pane)
                    .is_none_or(|pane| pane.handle.is_none())
            {
                self.spawn(pane, Some(command), session.working_dir().clone());
            }
            if activation.should_attach() {
                self.focus = Focus::Terminal;
            }
            return;
        }
        let pane = self.next_pane_id();
        let process = Process::builder()
            .id(pane)
            .name(session.name().clone())
            .kind(ProcessKind::Agent)
            .agent_tool(Some(*session.tool()))
            .agent_session_id(Some(session.id().clone()))
            .origin(ProcessOrigin::Session)
            .command(activation.command().cloned())
            .working_dir(session.working_dir().clone())
            .autostart(activation.command().is_some())
            .build();
        let selected = self.workspace.insert_in_section(process);
        self.workspace.select_at(selected);
        self.project_cursor = None;
        self.overlay = None;
        if let Some(command) = activation.command().cloned() {
            self.spawn(pane, Some(command), session.working_dir().clone());
        }
        if activation.should_attach() {
            self.focus = Focus::Terminal;
        }
    }

    /// Reopens a persisted session by ID with its provider-native command.
    pub(super) fn reopen_agent_session(&mut self, id: &AgentSessionId) {
        if self.pending_switch.is_some() {
            self.set_notice(PROJECT_SWITCH_IN_PROGRESS.to_string());
            return;
        }
        let session = match self.agent_sessions() {
            Ok(sessions) => sessions.into_iter().find(|session| session.id() == id),
            Err(error) => {
                self.set_notice(format!("{AGENT_SESSION_STORE_ERROR}: {error}"));
                return;
            },
        };
        let Some(session) = session else {
            self.set_notice(NO_RECENT_AGENT_SESSION.to_string());
            return;
        };
        let Some(command) = session.reopen_command() else {
            self.set_notice(AGENT_SESSION_NOT_RESUMABLE.to_string());
            return;
        };
        if let Some(store) = &self.agent_session_store
            && let Err(error) = store.set_state(session.id(), AgentSessionState::Open)
        {
            self.set_notice(format!("{AGENT_SESSION_STORE_ERROR}: {error}"));
            return;
        }
        let closing = self.workspace.processes().iter().find_map(|process| {
            (process.agent_session_id().as_ref() == Some(session.id()))
                .then_some(*process.id())
                .filter(|pane| {
                    self.panes.get(pane).is_some_and(|target| {
                        target.handle.is_some()
                            && target.config_membership == ConfigMembership::RetireOnExit
                    })
                })
        });
        if let Some(pane) = closing {
            self.pending_session_reopens
                .insert(pane, session.id().clone());
            return;
        }
        self.insert_agent_session(&session, AgentSessionActivation::StartAttached(command));
    }

    /// Reopens the newest closed resumable session owned by this workspace.
    pub(super) fn reopen_last_closed_session(&mut self) {
        let Some(project) = self.current_config.as_ref() else {
            return;
        };
        let sessions = match self.agent_sessions() {
            Ok(sessions) => sessions,
            Err(error) => {
                self.set_notice(format!("{AGENT_SESSION_STORE_ERROR}: {error}"));
                return;
            },
        };
        let belongs_to_project = |session: &AgentSession| {
            // Configured records are driven by muster.yml, not runtime history, so
            // `u` never reopens one as a disposable session.
            session.configured_key().is_none()
                && *session.state() == AgentSessionState::Closed
                && Self::same_config_location(session.project(), project)
        };
        let has_closed = sessions.iter().any(&belongs_to_project);
        let session = sessions
            .iter()
            .rev()
            .filter(|session| belongs_to_project(session))
            .find(|session| session.reopen_command().is_some());
        let Some(session) = session else {
            self.set_notice(
                if has_closed {
                    AGENT_SESSION_NOT_RESUMABLE
                } else {
                    NO_RECENT_AGENT_SESSION
                }
                .to_string(),
            );
            return;
        };
        let id = session.id().clone();
        self.reopen_agent_session(&id);
    }

    /// Loads durable session history, returning an empty list in test or custom
    /// compositions that deliberately omit the store.
    pub(super) fn agent_sessions(&self) -> Result<Vec<AgentSession>, ConfigError> {
        self.agent_session_store
            .as_ref()
            .map_or_else(|| Ok(Vec::new()), |store| store.sessions())
    }

    /// Returns a pane id unused by configured and runtime processes.
    pub(super) fn next_pane_id(&self) -> PaneId {
        let next = self
            .workspace
            .processes()
            .iter()
            .map(|process| process.id().into_inner())
            .max()
            .map_or(0, |pane| pane + 1);
        PaneId::new(next)
    }

    /// Adds a persistent terminal, command, or agent and reconciles it in place
    /// without interrupting existing configured processes or agent sessions.
    pub(super) fn add_configured_process(&mut self, kind: ProcessKind, values: &[String]) {
        let (Some(name_input), Some(command)) = (values.first(), values.get(1)) else {
            return;
        };
        let Some(config_path) = self.current_config.clone() else {
            return;
        };
        let Some(name) = self.resolve_configured_name(kind, name_input.trim()) else {
            return;
        };
        let command = command.trim();
        let command = if command.is_empty() {
            None
        } else {
            match CommandLine::try_new(command) {
                Ok(command) => Some(command),
                Err(_) => return,
            }
        };
        // A terminal with no command is a login shell by design; an agent with no
        // command would be that same shell mislabeled as an agent, so it is
        // rejected rather than persisted.
        if kind == ProcessKind::Agent && command.is_none() {
            self.report_error(AGENT_COMMAND_REQUIRED);
            return;
        }
        let spec = ProcessSpec::builder().name(name).command(command).build();
        let target = ProcessSpecMatcher::of_spec(kind, &spec);
        // Route through the registry's locked read-modify-write, the same one
        // `muster run` uses, so an overlapping CLI add and this add cannot
        // silently discard each other.
        let mut updated = None;
        let mut target_occurrence = None;
        let update_result = {
            let mut append = |config: WorkspaceConfig| {
                let config = match kind {
                    ProcessKind::Agent => {
                        let mut specs = config.agents().clone();
                        target_occurrence =
                            Some(specs.iter().filter(|spec| target.matches(spec)).count());
                        specs.push(spec.clone());
                        config.with_agents(specs)
                    },
                    ProcessKind::Terminal => {
                        let mut specs = config.terminals().clone();
                        target_occurrence =
                            Some(specs.iter().filter(|spec| target.matches(spec)).count());
                        specs.push(spec.clone());
                        config.with_terminals(specs)
                    },
                    ProcessKind::Command => {
                        let mut specs = config.commands().clone();
                        target_occurrence =
                            Some(specs.iter().filter(|spec| target.matches(spec)).count());
                        specs.push(spec.clone());
                        config.with_commands(specs)
                    },
                };
                updated = Some(config.clone());
                config
            };
            self.registry.update_workspace(&config_path, &mut append)
        };
        if update_result.is_err() {
            self.report_error(WORKSPACE_SAVE_ERROR);
            return;
        }
        let Some(config) = updated else {
            self.report_error(WORKSPACE_SAVE_ERROR);
            return;
        };
        self.overlay = None;
        self.reconcile_config(&config);
        let launch_pane = target_occurrence
            .and_then(|occurrence| self.configured_process_for_spec_occurrence(&target, occurrence))
            .filter(|process| *process.autostart() && !process.state().is_active())
            .map(|process| *process.id());
        if let Some(pane) = launch_pane {
            // Autostart the just-added agent with automatic (restore) semantics: a
            // fresh agent launches, a captured one resumes, and a re-added agent
            // whose reporter session was never captured is left stopped rather than
            // silently starting a new conversation. A failed link yields Ok(None).
            match self.command_of(pane, LaunchIntent::Automatic) {
                Ok(Some((command, cwd))) => self.spawn(pane, command, cwd),
                Ok(None) => {},
                Err(error) => self.set_notice(format!("{AGENT_SESSION_STORE_ERROR}: {error}")),
            }
        }
    }

    /// Returns the tracked configured process representing one occurrence of a
    /// spec identity after reconciliation.
    pub(super) fn configured_process_for_spec_occurrence(
        &self,
        target: &ProcessSpecMatcher,
        occurrence: usize,
    ) -> Option<&Process> {
        self.workspace
            .processes()
            .iter()
            .filter(|process| *process.origin() == ProcessOrigin::Configured)
            .filter(|process| target.matches_process(process))
            .filter(|process| {
                self.panes
                    .get(process.id())
                    .is_none_or(|pane| pane.config_membership == ConfigMembership::Tracked)
            })
            .nth(occurrence)
    }

    /// Handles a key while the switcher is open: navigate, jump by number,
    /// confirm the highlighted project, or cancel.
    pub(super) fn handle_switcher_key(&mut self, key: KeyEvent) {
        let Some(switcher) = self.switcher() else {
            return;
        };
        let count = switcher.projects.len();
        let selected = switcher.selected;
        match key.code {
            KeyCode::Esc => self.overlay = None,
            KeyCode::Char('j') | KeyCode::Down => {
                if let Some(switcher) = self.switcher_mut()
                    && count > 0
                {
                    switcher.selected = (selected + 1) % count;
                }
                self.update_switcher_preview();
            },
            KeyCode::Char('k') | KeyCode::Up => {
                if let Some(switcher) = self.switcher_mut()
                    && count > 0
                {
                    switcher.selected = if selected == 0 {
                        count - 1
                    } else {
                        selected - 1
                    };
                }
                self.update_switcher_preview();
            },
            KeyCode::Enter => {
                if count > 0 {
                    self.switch_to(selected);
                }
            },
            KeyCode::Char('n') => self.open_new_project_form(),
            KeyCode::Char('s') => self.open_save_project_form(),
            KeyCode::Char('a') => self.open_add_process_form(),
            KeyCode::Char('d') => self.remove_selected_project(),
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                let index = usize::from(c as u8 - b'1');
                if index < count {
                    self.switch_to(index);
                }
            },
            _ => {},
        }
    }
}
