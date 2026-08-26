use super::*;

impl App {
    /// Handles a command key pressed after the leader while a terminal is focused.
    pub(super) fn handle_leader_command(&mut self, key: KeyEvent) {
        if is_leader(key) {
            self.forward_key(key);
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.running = false,
            KeyCode::Char('h') | KeyCode::Left | KeyCode::Esc => self.focus = Focus::Sidebar,
            KeyCode::Char('j') | KeyCode::Down => self.workspace.select_next(),
            KeyCode::Char('k') | KeyCode::Up => self.workspace.select_previous(),
            KeyCode::Char('s') => self.toggle_selected(),
            KeyCode::Char('r') => self.restart_selected(),
            KeyCode::Char('p') => self.toggle_pause_selected(),
            KeyCode::Char('a') => self.open_add_process_form(),
            KeyCode::Char('A') => self.open_agent_picker(),
            KeyCode::Char('!') => {
                if self.jump_to_attention() {
                    self.focus = Focus::Terminal;
                }
            },
            KeyCode::Char('n') => self.open_new_project_form(),
            KeyCode::Char('N') => self.toggle_desktop_notifications(),
            KeyCode::Char('o') => self.open_switcher(),
            KeyCode::Char('x') => self.force_stop_selected(),
            KeyCode::Char('d') => self.confirm_delete_selected(),
            KeyCode::Char('u') => self.reopen_last_closed_session(),
            KeyCode::Char('?') => self.overlay = Some(Overlay::Help),
            _ => {},
        }
    }

    /// Opens the project switcher, reloading the registry so on-disk edits are
    /// picked up. Highlights the current project, else the first one.
    pub(super) fn open_switcher(&mut self) {
        let (projects, error) = match self.registry.projects() {
            Ok(projects) => (projects, None),
            Err(err) => (Vec::new(), Some(err.to_string())),
        };
        let current = self.current_project_index(&projects);
        let selected = current.unwrap_or(0);
        let preview = Switcher::preview(self.registry.as_ref(), projects.get(selected));
        self.overlay = Some(Overlay::Switcher(Switcher {
            projects,
            selected,
            current,
            error,
            preview,
        }));
    }

    /// Opens `form` with `intent`, retaining an open switcher for cancellation.
    pub(super) fn open_form(&mut self, form: Form, intent: FormIntent) {
        let switcher = match self.overlay.take() {
            Some(Overlay::Switcher(switcher)) => Some(switcher),
            Some(Overlay::Form(form)) => form.switcher,
            Some(Overlay::AgentPicker(picker)) => picker.switcher,
            _ => None,
        };
        self.overlay = Some(Overlay::Form(FormOverlay {
            modal: FormModal {
                form,
                intent,
                error: None,
            },
            switcher,
        }));
    }

    /// Opens the save-current-project form (one name field).
    pub(super) fn open_save_project_form(&mut self) {
        let form = Form::new(SAVE_PROJECT_TITLE, vec![Field::text(NAME_FIELD)]);
        self.open_form(form, FormIntent::SaveCurrentProject);
    }

    /// Opens the new-project form: a name, and a folder prefilled with the
    /// current directory so the common case is just typing a name.
    pub(super) fn open_new_project_form(&mut self) {
        let folder = std::env::current_dir()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        let form = Form::new(NEW_PROJECT_TITLE, vec![
            Field::text(NAME_FIELD),
            Field::path(FOLDER_FIELD, &folder),
        ]);
        self.open_form(form, FormIntent::NewProject);
    }

    /// Opens the first add-process step, where the process kind is selected.
    pub(super) fn open_add_process_form(&mut self) {
        let form = Form::new(ADD_PROCESS_TITLE, vec![Field::choice(
            KIND_FIELD,
            &KIND_OPTIONS,
        )]);
        self.open_form(form, FormIntent::ChooseProcessKind);
    }

    /// Opens the quick agent picker with resumable history and fresh presets.
    pub(super) fn open_agent_picker(&mut self) {
        let project = self.current_config.clone();
        let (mut items, error) = match self.agent_sessions() {
            Ok(sessions) => (
                sessions
                    .into_iter()
                    .rev()
                    .filter(|session| {
                        // Configured records are muster.yml agents, not runtime
                        // history, so they never appear as recent conversations.
                        session.configured_key().is_none()
                            && *session.state() == AgentSessionState::Closed
                            && session.reopen_command().is_some()
                            && project.as_ref().is_some_and(|project| {
                                Self::same_config_location(session.project(), project)
                            })
                    })
                    .take(RECENT_AGENT_LIMIT)
                    .map(Box::new)
                    .map(AgentPickerItem::Recent)
                    .collect::<Vec<_>>(),
                None,
            ),
            Err(error) => (Vec::new(), Some(error.to_string())),
        };
        items.extend(AgentTool::options().map(AgentPickerItem::New));
        // Reached with `A`, never from the switcher, so nothing to retain behind it.
        self.overlay = Some(Overlay::AgentPicker(AgentPicker {
            items,
            selected: 0,
            error,
            purpose: AgentPickerPurpose::Launch,
            switcher: None,
        }));
    }

    /// Opens the configured-agent flow's provider menu: the same picker as `A` but
    /// listing only presets (plus Custom) and creating a persisted `muster.yml`
    /// agent rather than a disposable session. Retains any switcher held behind the
    /// kind form so Esc returns to it, matching the terminal and command branches.
    pub(super) fn open_configured_agent_picker(&mut self) {
        let switcher = match self.overlay.take() {
            Some(Overlay::Switcher(switcher)) => Some(switcher),
            Some(Overlay::Form(form)) => form.switcher,
            Some(Overlay::AgentPicker(picker)) => picker.switcher,
            _ => None,
        };
        let mut items: Vec<AgentPickerItem> =
            AgentTool::options().map(AgentPickerItem::New).collect();
        // The last row drills into existing conversations to pin one as a native,
        // persisted agent - which is how a conversation from another folder becomes
        // a member of this workspace, keeping its own cwd.
        items.push(AgentPickerItem::FromConversation);
        self.overlay = Some(Overlay::AgentPicker(AgentPicker {
            items,
            selected: 0,
            error: None,
            purpose: AgentPickerPurpose::Configure,
            switcher,
        }));
    }

    /// Opens the name step for a configured agent of `tool`. Only the name is
    /// asked; the launch command comes from the preset. A blank name
    /// autogenerates a project-unique one.
    pub(super) fn open_configured_agent_name_form(&mut self, tool: AgentTool) {
        let form = Form::new(ADD_PROCESS_TITLE, vec![Field::text(SESSION_NAME_FIELD)]);
        self.open_form(form, FormIntent::AddConfiguredAgent(tool));
    }

    /// Lists every resumable conversation - across all folders - so one can be
    /// pinned into this workspace as a configured agent. Only conversations with a
    /// captured provider identity appear, since only those can be resumed. Retains
    /// the switcher held behind the configure menu so Esc unwinds to it.
    pub(super) fn open_pin_conversation_picker(&mut self) {
        let switcher = match self.overlay.take() {
            Some(Overlay::Switcher(switcher)) => Some(switcher),
            Some(Overlay::Form(form)) => form.switcher,
            Some(Overlay::AgentPicker(picker)) => picker.switcher,
            _ => None,
        };
        let (items, error) = match self.agent_sessions() {
            Ok(sessions) => (
                sessions
                    .into_iter()
                    .rev()
                    .filter(|session| {
                        // Offer only a settled conversation that can actually be
                        // resumed. A configured record is a muster.yml agent, not
                        // history (and a pinned conversation is already one, so it is
                        // never offered twice). `resume` is `Some` only with a
                        // captured identity and a constructible resume command, so a
                        // wrapped or template-less session that could not autostart is
                        // filtered out rather than pinned into a dead agent. A just
                        // deleted agent whose process is still shutting down is closed
                        // and un-configured, but its owner is still live, so it is held
                        // back until the provider exits - pinning it now would transfer
                        // a conversation out from under a running provider.
                        session.configured_key().is_none()
                            && *session.state() == AgentSessionState::Closed
                            && session.resume().is_some()
                            && !Self::session_owner_is_live(session)
                    })
                    .map(Box::new)
                    .map(AgentPickerItem::Conversation)
                    .collect(),
                None,
            ),
            Err(error) => (Vec::new(), Some(error.to_string())),
        };
        self.overlay = Some(Overlay::AgentPicker(AgentPicker {
            items,
            selected: 0,
            error,
            purpose: AgentPickerPurpose::PinConversation,
            switcher,
        }));
    }

    /// Opens the name step for pinning `source` as a configured agent. A blank
    /// name autogenerates a project-unique one; the folder and resume identity come
    /// from the conversation.
    pub(super) fn open_pin_conversation_name_form(&mut self, source: Box<AgentSession>) {
        let form = Form::new(ADD_PROCESS_TITLE, vec![Field::text(SESSION_NAME_FIELD)]);
        self.open_form(form, FormIntent::PinConversation(source));
    }

    /// Handles navigation, quick launch, customization, and cancel in the agent
    /// picker.
    pub(super) fn handle_agent_picker_key(&mut self, key: KeyEvent) {
        let Some(Overlay::AgentPicker(picker)) = &self.overlay else {
            return;
        };
        let count = picker.items.len();
        let selected = picker.selected;
        let purpose = picker.purpose;
        match key.code {
            // The pin list is drilled into from the configure provider menu, so Esc
            // steps back to that menu rather than unwinding past it to the switcher.
            KeyCode::Esc if purpose == AgentPickerPurpose::PinConversation => {
                self.open_configured_agent_picker();
            },
            KeyCode::Esc => self.close_overlay(),
            KeyCode::Char('j') | KeyCode::Down if count > 0 => {
                if let Some(Overlay::AgentPicker(picker)) = &mut self.overlay {
                    picker.selected = (selected + 1) % count;
                }
            },
            KeyCode::Char('k') | KeyCode::Up if count > 0 => {
                if let Some(Overlay::AgentPicker(picker)) = &mut self.overlay {
                    picker.selected = selected.checked_sub(1).unwrap_or(count - 1);
                }
            },
            KeyCode::Enter => {
                let item = match &self.overlay {
                    Some(Overlay::AgentPicker(picker)) => picker.items.get(selected).cloned(),
                    _ => None,
                };
                match (purpose, item) {
                    (_, None) => {},
                    (AgentPickerPurpose::Launch, Some(AgentPickerItem::Recent(session))) => {
                        self.overlay = None;
                        self.reopen_agent_session(session.id());
                    },
                    (AgentPickerPurpose::Launch, Some(AgentPickerItem::New(AgentTool::Custom))) => {
                        self.open_agent_session_form(AgentTool::Custom);
                    },
                    (AgentPickerPurpose::Launch, Some(AgentPickerItem::New(tool))) => {
                        self.overlay = None;
                        self.create_agent_session(tool, None, None, None);
                    },
                    (
                        AgentPickerPurpose::Configure,
                        Some(AgentPickerItem::New(AgentTool::Custom)),
                    ) => {
                        // Custom drops to the command panel, where the command is
                        // typed and the provider inferred from it.
                        self.open_configured_process_form(ProcessKind::Agent);
                    },
                    (AgentPickerPurpose::Configure, Some(AgentPickerItem::New(tool))) => {
                        self.open_configured_agent_name_form(tool);
                    },
                    (AgentPickerPurpose::Configure, Some(AgentPickerItem::FromConversation)) => {
                        self.open_pin_conversation_picker();
                    },
                    (
                        AgentPickerPurpose::PinConversation,
                        Some(AgentPickerItem::Conversation(session)),
                    ) => {
                        self.open_pin_conversation_name_form(session);
                    },
                    // Combinations a purpose never surfaces are enumerated rather than
                    // caught by a wildcard, so a new item or purpose that needs
                    // wiring fails to compile here instead of silently no-opping.
                    (
                        AgentPickerPurpose::Launch,
                        Some(AgentPickerItem::FromConversation | AgentPickerItem::Conversation(_)),
                    ) => {},
                    (
                        AgentPickerPurpose::Configure,
                        Some(AgentPickerItem::Recent(_) | AgentPickerItem::Conversation(_)),
                    ) => {},
                    (
                        AgentPickerPurpose::PinConversation,
                        Some(
                            AgentPickerItem::Recent(_)
                            | AgentPickerItem::New(_)
                            | AgentPickerItem::FromConversation,
                        ),
                    ) => {},
                }
            },
            KeyCode::Char('e') if purpose == AgentPickerPurpose::Launch => {
                let tool = match &self.overlay {
                    Some(Overlay::AgentPicker(picker)) => picker.items.get(selected),
                    _ => None,
                };
                if let Some(AgentPickerItem::New(tool)) = tool {
                    self.open_agent_session_form(*tool);
                }
            },
            _ => {},
        }
    }

    /// Opens advanced customization for a fresh agent provider.
    pub(super) fn open_agent_session_form(&mut self, tool: AgentTool) {
        let tool_options = AgentTool::options()
            .map(|tool| tool.to_string())
            .collect::<Vec<_>>();
        let tool_option_refs = tool_options.iter().map(String::as_str).collect::<Vec<_>>();
        let form = Form::new(ADD_AGENT_TITLE, vec![
            Field::choice_value(TOOL_FIELD, &tool_option_refs, tool.as_ref()),
            Field::text(SESSION_NAME_FIELD),
            Field::text(AGENT_COMMAND_FIELD),
            Field::text(AGENT_RESUME_FIELD),
        ]);
        self.open_form(form, FormIntent::LaunchAgentSession);
    }

    /// Opens the persistent agent, terminal, or command form. An agent's name is
    /// optional: left blank, muster autogenerates a project-unique one, since the
    /// name is the agent's durable session key. Terminals and commands require a
    /// name.
    pub(super) fn open_configured_process_form(&mut self, kind: ProcessKind) {
        let name_field = if kind == ProcessKind::Agent {
            SESSION_NAME_FIELD
        } else {
            NAME_FIELD
        };
        let form = Form::new(ADD_PROCESS_TITLE, vec![
            Field::text(name_field),
            Field::text(COMMAND_FIELD),
        ]);
        self.open_form(form, FormIntent::AddConfiguredProcess(kind));
    }

    /// Removes the highlighted project from the registry. The switcher row
    /// only names the target; the removal itself runs against a freshly read
    /// list under the registry lock, so a stale snapshot never overwrites
    /// concurrent registry changes.
    pub(super) fn remove_selected_project(&mut self) {
        let Some(switcher) = self.switcher() else {
            return;
        };
        let Some(target) = switcher.projects.get(switcher.selected) else {
            return;
        };
        let target_config = target.config().clone();
        let result = self.registry.update_projects(&mut |mut projects| {
            projects
                .retain(|project| !Self::same_config_location(project.config(), &target_config));
            projects
        });
        match result {
            Ok(()) => {
                self.refresh_projects();
                self.refresh_switcher();
            },
            Err(error) => {
                if let Some(switcher) = self.switcher_mut() {
                    switcher.error = Some(error.to_string());
                }
            },
        }
    }

    /// Reloads the open switcher from the registry after a change.
    pub(super) fn refresh_switcher(&mut self) {
        if self.switcher().is_none() {
            return;
        }
        let (projects, error) = match self.registry.projects() {
            Ok(projects) => (projects, None),
            Err(err) => (Vec::new(), Some(err.to_string())),
        };
        let current = self.current_project_index(&projects);
        let selected = current.unwrap_or(0).min(projects.len().saturating_sub(1));
        let preview = Switcher::preview(self.registry.as_ref(), projects.get(selected));
        self.overlay = Some(Overlay::Switcher(Switcher {
            projects,
            selected,
            current,
            error,
            preview,
        }));
    }

    /// Recomputes the open switcher's cached preview for its current selection,
    /// after the highlight moves.
    pub(super) fn update_switcher_preview(&mut self) {
        let Some(project) = self
            .switcher()
            .and_then(|switcher| switcher.projects.get(switcher.selected).cloned())
        else {
            return;
        };
        let preview = Switcher::preview(self.registry.as_ref(), Some(&project));
        if let Some(switcher) = self.switcher_mut() {
            switcher.preview = preview;
        }
    }
}
