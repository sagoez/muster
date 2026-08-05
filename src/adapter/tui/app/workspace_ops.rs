use super::*;

impl App {
    /// Handles a project whose config could not be opened: if the file is gone,
    /// offer to remove the stale entry right away; if it is present but
    /// unreadable, just report it (removing an entry whose file still exists
    /// would be destructive).
    pub(super) fn report_project_open_failure(&mut self, project: &Project, err: &ConfigError) {
        if self.registry.workspace_exists(project.config()) {
            self.set_notice(format!("{}: {err}", project.name().as_ref()));
            return;
        }
        let message = format!("{}'s config file is missing.", project.name().as_ref());
        self.confirm_remove_project(project, message);
    }

    /// Opens a confirmation to remove `project` from the registry, closing any
    /// project overlay first. Shared by activation failures and the sidebar `d`.
    pub(super) fn confirm_remove_project(&mut self, project: &Project, message: String) {
        self.project_cursor = None;
        self.overlay = Some(Overlay::ConfirmRemoval {
            message,
            config_path: project.config().clone(),
        });
    }

    /// Confirms removal of the project on the selected sidebar row.
    pub(super) fn confirm_remove_selected_project(&mut self) {
        let Some(cursor) = self.project_cursor else {
            return;
        };
        let Some(project) = self
            .other_projects()
            .get(cursor)
            .and_then(|index| self.projects.get(*index))
            .cloned()
        else {
            return;
        };
        // The synthetic launched-project row has no registry entry: "removing" it
        // would save an unchanged list and immediately reappear, so refuse.
        if self.is_synthetic_launched(&project) {
            self.set_notice(CANNOT_REMOVE_LAUNCHED.to_string());
            return;
        }
        let message = format!("Remove project '{}'?", project.name().as_ref());
        self.confirm_remove_project(&project, message);
    }

    /// Confirms closing the selected runtime agent session. A configured
    /// agent remains pinned because its lifecycle is owned by `muster.yml`.
    pub(super) fn confirm_close_selected_session(&mut self) {
        let Some(process) = self.workspace.selected_process() else {
            return;
        };
        if *process.kind() != ProcessKind::Agent {
            return;
        }
        if *process.origin() != ProcessOrigin::Session {
            self.set_notice(CONFIGURED_AGENT_CLOSE_UNAVAILABLE.to_string());
            return;
        }
        let pane = *process.id();
        let message = format!("Close agent session {}?", process.name().as_ref());
        if self
            .panes
            .get(&pane)
            .is_some_and(|target| target.handle.is_some())
        {
            self.overlay = Some(Overlay::ConfirmSessionClose { message, pane });
        } else {
            self.close_agent_session(pane);
        }
    }

    /// Force-kills one runtime agent session and records it closed only after
    /// stop delivery succeeds. Its row retires on exit, or immediately when it
    /// was already stopped, with focus returned to the sidebar first.
    pub(super) fn close_agent_session(&mut self, pane: PaneId) {
        self.focus = Focus::Sidebar;
        let session_id = self
            .workspace
            .process(pane)
            .and_then(|process| process.agent_session_id().clone());
        let alive = self
            .panes
            .get(&pane)
            .is_some_and(|target| target.handle.is_some());
        if !alive {
            if self.persist_agent_session_state(session_id.as_ref(), AgentSessionState::Closed) {
                self.pending_session_reopens.remove(&pane);
                self.retire_pane(pane);
            }
            return;
        }
        let delivered = self
            .panes
            .get_mut(&pane)
            .and_then(|target| target.handle.as_mut().map(|handle| handle.kill().is_ok()))
            .unwrap_or(false);
        if !delivered {
            self.set_notice(STOP_DELIVERY_FAILED_NOTICE.to_string());
            return;
        }
        self.workspace.set_state(pane, ProcessState::Stopping);
        if !self.persist_agent_session_state(session_id.as_ref(), AgentSessionState::Closed) {
            return;
        }
        self.pending_session_reopens.remove(&pane);
        if let Some(target) = self.panes.get_mut(&pane) {
            target.config_membership = ConfigMembership::RetireOnExit;
        }
    }

    /// Persists one session lifecycle transition and exposes adapter failures
    /// without allowing the in-memory transition to continue.
    pub(super) fn persist_agent_session_state(
        &mut self,
        session_id: Option<&AgentSessionId>,
        state: AgentSessionState,
    ) -> bool {
        let (Some(store), Some(session_id)) = (&self.agent_session_store, session_id) else {
            return true;
        };
        if let Err(error) = store.set_state(session_id, state) {
            self.set_notice(format!("{AGENT_SESSION_STORE_ERROR}: {error}"));
            self.overlay = None;
            return false;
        }
        true
    }

    /// Whether `project` is the unsaved launched-project row synthesized for the
    /// tree rather than a registered project.
    pub(super) fn is_synthetic_launched(&self, project: &Project) -> bool {
        self.launched_project_membership == LaunchedProjectMembership::Synthetic
            && Self::same_config_location(project.config(), &self.launched_config)
    }

    /// Flips the selected process's autostart on or off. The explicit value is
    /// written to the matching spec first, and the live process is updated only
    /// when that write both succeeds and actually found a spec to change, so the
    /// sidebar never shows a state the config did not record. The spec is located
    /// by the process's full resolved identity, and among identical rows by the
    /// selected one's position within them, so the persisted change lands on the
    /// row the user picked whatever order a reconcile left the rows in.
    pub(super) fn toggle_selected_autostart(&mut self) {
        let Some(config_path) = self.current_config.clone() else {
            return;
        };
        let Some(process) = self.workspace.selected_process() else {
            return;
        };
        if *process.origin() == ProcessOrigin::Session {
            self.set_notice(SESSION_AUTOSTART_UNAVAILABLE.to_string());
            return;
        }
        let pane = *process.id();
        let autostart = !*process.autostart();
        let target = ProcessSpecMatcher::of(process);
        let occurrence = self
            .workspace
            .processes()
            .iter()
            .filter(|candidate| *candidate.origin() == ProcessOrigin::Configured)
            .filter(|candidate| target.matches_process(candidate))
            .position(|candidate| *candidate.id() == pane)
            .unwrap_or(0);

        let mut edited = false;
        let mut apply = |config: WorkspaceConfig| {
            let (config, found) = target.with_autostart(config, occurrence, Some(autostart));
            edited = found;
            config
        };
        match self.registry.update_workspace(&config_path, &mut apply) {
            Ok(()) if edited => self.workspace.set_autostart(pane, autostart),
            Ok(()) => self.set_notice(AUTOSTART_UNTRACKED.to_string()),
            Err(_) => self.set_notice(WORKSPACE_SAVE_ERROR.to_string()),
        }
    }

    /// Removes the registered project at `config_path` from the registry,
    /// under the registry lock and against a freshly read list so concurrent
    /// CLI mutations are never overwritten with stale state.
    pub(super) fn remove_project(&mut self, config_path: &Path) {
        let result = self.registry.update_projects(&mut |mut projects| {
            projects.retain(|project| !Self::same_config_location(project.config(), config_path));
            projects
        });
        match result {
            Ok(()) => {
                self.project_cursor = None;
                self.refresh_projects();
            },
            Err(error) => self.set_notice(error.to_string()),
        }
    }

    /// The active project's display name: its registered name, else the config's
    /// parent directory, else the app name - so its header is never blank.
    pub(super) fn active_project_label(&self) -> String {
        if let Some(index) = self.current_project_index(&self.projects)
            && let Some(project) = self.projects.get(index)
        {
            return project.name().as_ref().to_string();
        }
        self.current_config
            .as_deref()
            .map(label_from_config)
            .unwrap_or_else(|| APP_NAME.to_string())
    }
}
