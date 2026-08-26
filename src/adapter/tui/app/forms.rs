use super::*;

/// Whether registering a project scaffolds a starter `muster.yml` or adopts an
/// existing one as-is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConfigDisposition {
    /// A brand-new folder with no config: write the starter config.
    Scaffold,
    /// An existing `muster.yml` the user confirmed: adopt it, write nothing.
    Adopt,
}

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
        let intent = modal.intent.clone();
        match intent {
            FormIntent::SaveCurrentProject => self.save_current_project(&values),
            FormIntent::NewProject => self.new_project(&values),
            FormIntent::ChooseProcessKind => self.choose_process_kind(&values),
            FormIntent::LaunchAgentSession => self.launch_agent_session(&values),
            FormIntent::AddConfiguredProcess(kind) => {
                self.add_configured_process(kind, &values);
            },
            FormIntent::AddConfiguredAgent(tool) => self.add_configured_agent(tool, &values),
            FormIntent::PinConversation(source) => {
                self.pin_conversation(&source, values.first().map_or("", String::as_str));
            },
        }
    }

    /// Persists a configured agent for a chosen provider preset. The form supplies
    /// only the (optional) name; the launch command is the preset's default, so
    /// this reuses [`Self::add_configured_process`] with that command filled in.
    pub(super) fn add_configured_agent(&mut self, tool: AgentTool, values: &[String]) {
        let Some(name_input) = values.first() else {
            return;
        };
        // Only presets reach here; Custom routes to the command panel instead.
        let Some(command) = tool.default_command() else {
            self.report_error(AGENT_COMMAND_REQUIRED);
            return;
        };
        self.add_configured_process(ProcessKind::Agent, &[
            name_input.clone(),
            command.to_string(),
        ]);
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
        if self
            .try_register(Project::builder().name(name).config(config).build())
            .is_some()
        {
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
        self.create_project(name, config_path, ConfigDisposition::Scaffold);
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

    /// Registers the project, then writes a starter config only when scaffolding a
    /// brand-new folder. For a brand-new folder, registration comes first so a
    /// failed write leaves a recoverable dangling entry (a retry heals it) rather
    /// than a stranded file that would block re-creation. Adopting an existing
    /// `muster.yml` instead validates it first by loading it, so a path that is a
    /// directory, has vanished, or is unreadable never registers a project that
    /// cannot be opened; the file is then used as-is and never rewritten.
    pub(super) fn create_project(
        &mut self,
        name: ProjectName,
        config_path: PathBuf,
        disposition: ConfigDisposition,
    ) {
        // Adoption must confirm the config is readable now, not merely that a path
        // existed when the form opened: it may be a directory, have vanished while
        // the confirmation was open, or be otherwise unreadable. Load it before
        // registering so a project that cannot be opened is never created.
        if disposition == ConfigDisposition::Adopt
            && let Err(error) = self.registry.workspace(&config_path)
        {
            self.report_error(&error.to_string());
            return;
        }
        let project = Project::builder()
            .name(name.clone())
            .config(config_path.clone())
            .build();
        let Some(preimage) = self.try_register(project) else {
            return;
        };
        // Scaffold only a brand-new folder. `create_workspace` is an atomic exclusive
        // create - it never overwrites an existing file and needs no check-then-write
        // that could clobber a config another process wrote in between. Adoption skips
        // it: the file already exists, so a write would be redundant and would fail
        // needlessly in a folder muster cannot write to.
        if disposition == ConfigDisposition::Scaffold {
            match self
                .registry
                .create_workspace(&config_path, &starter_workspace())
            {
                // Wrote a fresh starter config; it is valid by construction.
                Ok(true) => {},
                // Another process created the destination between the existence check
                // and this exclusive create. Adopt the winner, but validate it first:
                // a malformed, unreadable, or directory path must not close the form
                // as a success and leave an unopenable project.
                Ok(false) => {
                    if let Err(error) = self.registry.workspace(&config_path) {
                        // Registration ran before this create, but the path now exists
                        // and is unreadable, so a retry only re-enters the adoption path
                        // and is rejected again - the dangling entry can never heal.
                        // Roll it back so no permanently unopenable project is left,
                        // restoring whatever entry this registration replaced.
                        self.rollback_registration(&name, &config_path, preimage);
                        self.report_error(&error.to_string());
                        return;
                    }
                },
                Err(_) => {
                    self.report_error(WORKSPACE_SAVE_ERROR);
                    return;
                },
            }
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
                        self.create_project(name, config_path, ConfigDisposition::Adopt);
                    },
                    Some(Overlay::ConfirmRemoval { config_path, .. }) => {
                        self.remove_project(&config_path);
                    },
                    Some(Overlay::ConfirmSessionClose { pane, .. }) => {
                        self.close_agent_session(pane);
                    },
                    Some(Overlay::ConfirmProcessRemoval {
                        target,
                        occurrence,
                        config_path,
                        ..
                    }) => {
                        self.remove_configured_process(&target, occurrence, &config_path);
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
    /// path, atomically under the registry lock. On success returns the entries it
    /// replaced (the preimage, usually empty) so a caller can restore them if a
    /// later step fails; returns `None` and sets a form error when the write fails.
    /// Leaves the form open so the caller decides when to close it.
    pub(super) fn try_register(&mut self, project: Project) -> Option<Vec<Project>> {
        let project = Project::builder()
            .name(project.name().clone())
            .config(path::absolutize(project.config()))
            .build();
        let mut preimage = Vec::new();
        let result = self.registry.update_projects(&mut |mut projects| {
            // Capture and replace any existing entry at this location under the lock,
            // so a rollback can put the prior registration back verbatim.
            preimage.clear();
            projects.retain(|existing| {
                let replaced = Self::same_config_location(existing.config(), project.config());
                if replaced {
                    preimage.push(existing.clone());
                }
                !replaced
            });
            projects.push(project.clone());
            projects
        });
        match result {
            Ok(()) => Some(preimage),
            Err(error) => {
                self.report_error(&error.to_string());
                None
            },
        }
    }

    /// Undoes a `try_register` whose follow-up step failed, restoring the captured
    /// `preimage`. Removes only our own write (matched by name and location) so a
    /// registration another writer made in the meantime is preserved, and puts the
    /// replaced entries back so a failed scaffold never deletes existing registry
    /// data. Any error is swallowed: the caller already reports the original one.
    pub(super) fn rollback_registration(
        &mut self,
        name: &ProjectName,
        config_path: &Path,
        preimage: Vec<Project>,
    ) {
        // `try_register` stored the absolutized path; match on the same form or a
        // relative input would never find its own entry.
        let config_path = path::absolutize(config_path);
        let _ = self.registry.update_projects(&mut |mut projects| {
            let ours = projects.iter().position(|project| {
                project.name() == name && Self::same_config_location(project.config(), &config_path)
            });
            if let Some(index) = ours {
                projects.remove(index);
                projects.extend(preimage.iter().cloned());
            }
            projects
        });
        self.refresh_projects();
    }

    /// Advances the add flow to the selected kind's next step. The agent kind opens
    /// the provider menu (presets, or Custom for a typed command); both persist to
    /// `muster.yml` with autostart controllable by `t`. Terminals and commands go
    /// straight to their name-and-command form. The disposable-session picker is a
    /// separate flow reached with `A`.
    pub(super) fn choose_process_kind(&mut self, values: &[String]) {
        match values.first().map(String::as_str) {
            Some(KIND_AGENT) => self.open_configured_agent_picker(),
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
        self.persist_configured_process(kind, name, command, None);
    }

    /// Persists a resolved configured process to `muster.yml` and reconciles it in
    /// place. `working_dir` pins the process to a directory other than the
    /// workspace folder, which is how an agent pulled in from another folder keeps
    /// its own cwd.
    pub(super) fn persist_configured_process(
        &mut self,
        kind: ProcessKind,
        name: ProcessName,
        command: Option<CommandLine>,
        working_dir: Option<PathBuf>,
    ) {
        let spec = ProcessSpec::builder()
            .name(name)
            .command(command)
            .working_dir(working_dir)
            .build();
        let Some((config, target, occurrence)) = self.write_configured_spec(kind, &spec) else {
            return;
        };
        self.reconcile_configured_launch(&target, &config, occurrence);
    }

    /// Appends `spec` to `muster.yml` through the registry's locked
    /// read-modify-write - the same one `muster run` uses, so an overlapping CLI
    /// add cannot silently discard this one. Returns the new config, the spec
    /// matcher, and the occurrence index of the appended entry, or `None` (after
    /// reporting) when the write fails.
    fn write_configured_spec(
        &mut self,
        kind: ProcessKind,
        spec: &ProcessSpec,
    ) -> Option<(WorkspaceConfig, ProcessSpecMatcher, usize)> {
        let config_path = self.current_config.clone()?;
        let target = ProcessSpecMatcher::of_spec(kind, spec);
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
        match (update_result, updated, target_occurrence) {
            (Ok(()), Some(config), Some(occurrence)) => Some((config, target, occurrence)),
            _ => {
                self.report_error(WORKSPACE_SAVE_ERROR);
                None
            },
        }
    }

    /// Reconciles a just-written config in place and autostarts the appended
    /// process. Agents launch with automatic (restore) semantics: a fresh agent
    /// launches, a captured one resumes, and a re-added agent whose reporter
    /// session was never captured is left stopped rather than silently starting a
    /// new conversation. A failed link yields `Ok(None)`.
    fn reconcile_configured_launch(
        &mut self,
        target: &ProcessSpecMatcher,
        config: &WorkspaceConfig,
        occurrence: usize,
    ) {
        self.overlay = None;
        self.reconcile_config(config);
        let launch_pane = self
            .configured_process_for_spec_occurrence(target, occurrence)
            .filter(|process| *process.autostart() && !process.state().is_active())
            .map(|process| *process.id());
        if let Some(pane) = launch_pane {
            match self.command_of(pane, LaunchIntent::Automatic) {
                Ok(Some((command, cwd))) => self.spawn(pane, command, cwd),
                Ok(None) => {},
                Err(error) => self.set_notice(format!("{AGENT_SESSION_STORE_ERROR}: {error}")),
            }
        }
    }

    /// Pins an existing conversation as a configured agent in the current
    /// workspace: a native, autostarting member that resumes that exact
    /// conversation and keeps running in its own folder, even though this
    /// workspace lives elsewhere. A blank name autogenerates one.
    // TODO(pin-atomicity): this is a two-store transaction (the session transfer
    // in `pin_configured`, then the `muster.yml` write) that is not atomic across
    // processes. Two known cross-process races remain, both requiring two muster
    // instances on the same project pinning concurrently, or a crash between the
    // two writes:
    //   1. Concurrent pins to the same name can displace each other's transferred
    //      record before either config write wins, binding an agent to the wrong
    //      conversation or discarding a record.
    //   2. `recover_orphaned_pins` on one instance's startup cannot tell another
    //      instance's in-flight pin from an abandoned one, so it can un-configure a
    //      live transaction.
    // Fix: a durable pending-pin lease on the session holding the initiating
    // instance's process identity (reuse `LocalProcessIdentity`). `pin_configured`
    // sets it on transfer and rejects a target already held by a *live* lease; a
    // `commit_pin` step clears it after the config write; recovery un-configures an
    // orphan only when its lease is absent or its holder is dead. This is a
    // state-file schema change, deferred until the concurrency is worth the cost.
    pub(super) fn pin_conversation(&mut self, source: &AgentSession, name_input: &str) {
        let Some(project) = self.current_config.clone() else {
            return;
        };
        // A conversation whose provider is still running here must not be pinned: the
        // transfer would move its association out from under the live process. This
        // backstops the picker filter against a race between listing and confirming.
        if Self::session_owner_is_live(source) {
            self.report_error(PIN_SOURCE_STILL_RUNNING);
            return;
        }
        let Some(native_id) = source.native_id().clone() else {
            self.set_notice(AGENT_SESSION_NOT_RESUMABLE.to_string());
            return;
        };
        let Some(name) = self.resolve_configured_name(ProcessKind::Agent, name_input.trim()) else {
            return;
        };
        let Ok(key) = ConfiguredAgentKey::of(&name) else {
            return;
        };
        let command = source.launch_command().clone();
        let folder = Self::conversation_folder(source);
        // Transfer the source conversation into the configured agent's durable
        // session, reusing its id so the source history record is retired rather
        // than cloned: the native identity keeps a single launchable owner instead
        // of leaving the source reopenable from its own workspace in parallel. The
        // tool is re-inferred from the command it will run under, matching the tool
        // the linker later derives, so the reconcile refresh keeps the native id
        // instead of wiping it; the resume template is carried over so a
        // conversation resumed by a custom invocation keeps resuming that way.
        let seeded = AgentSession::builder()
            .id(source.id().clone())
            .name(name.clone())
            .tool(AgentTool::from_command(Some(&command)))
            .project(project.clone())
            .launch_command(command.clone())
            .working_dir(folder.clone())
            .resume_command(source.resume_command().clone())
            .native_id(Some(native_id))
            .configured_key(Some(key))
            .state(AgentSessionState::Closed)
            .build();
        // Adopt the identity before publishing the config, so the agent is never
        // written to muster.yml without its durable session (which would autostart a
        // fresh conversation). A store failure here touches nothing else; on success
        // it returns the records it displaced so they can be restored on rollback.
        let pinned = self
            .agent_session_store
            .as_ref()
            .map(|store| store.pin_configured(&seeded));
        let displaced = match pinned {
            Some(Ok(displaced)) => displaced,
            Some(Err(error)) => {
                self.report_error(&format!("{AGENT_SESSION_STORE_ERROR}: {error}"));
                return;
            },
            None => return,
        };
        let spec = ProcessSpec::builder()
            .name(name)
            .command(Some(command))
            .working_dir(folder)
            .build();
        let target = ProcessSpecMatcher::of_spec(ProcessKind::Agent, &spec);
        let Some((config, _, occurrence)) = self.write_configured_spec(ProcessKind::Agent, &spec)
        else {
            self.roll_back_pin(&project, &target, &seeded, &displaced);
            return;
        };
        self.reconcile_configured_launch(&target, &config, occurrence);
    }

    /// Rolls back a pin whose config write failed, restoring the records the
    /// transfer displaced so the conversation is not stranded as a configured
    /// record hidden from both pickers.
    ///
    /// The rollback is conditional: if an agent matching this pin's exact spec is
    /// now present in the config, a concurrent instance committed this same pin and
    /// owns the configured record, so restoring the pre-pin snapshot would clobber
    /// its live state. Matching the full spec (name and command), not just the name,
    /// avoids mistaking an unrelated agent that merely shares the name for this
    /// pin's commit. Only a genuinely orphaned transfer is undone. A restore that
    /// itself fails is surfaced rather than dropped, since it leaves the session
    /// store inconsistent with `muster.yml`.
    fn roll_back_pin(
        &mut self,
        config_path: &Path,
        target: &ProcessSpecMatcher,
        seeded: &AgentSession,
        displaced: &[AgentSession],
    ) {
        let committed = self
            .registry
            .workspace(config_path)
            .is_ok_and(|config| config.agents().iter().any(|agent| target.matches(agent)));
        if committed {
            return;
        }
        let restored = self
            .agent_session_store
            .as_ref()
            .map(|store| store.restore_sessions(seeded, displaced));
        if let Some(Err(error)) = restored {
            self.report_error(&format!("{AGENT_SESSION_ROLLBACK_ERROR}: {error}"));
        }
    }

    /// The absolute directory a conversation ran in, resolved the same way a spawn
    /// resolves its cwd: its working dir when set (absolutized against its own
    /// workspace folder if relative), else that workspace folder. `None` only when
    /// its config path has no parent directory.
    fn conversation_folder(source: &AgentSession) -> Option<PathBuf> {
        Self::resolve_spawn_paths(
            Some(source.project().as_path()),
            source.working_dir().clone(),
        )
        .1
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
