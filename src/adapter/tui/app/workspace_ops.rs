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

    /// On a process row, `d` either deletes a configured process from
    /// `muster.yml` (so a pinned agent stops coming back on the next reconcile)
    /// or closes a disposable runtime agent session. Configured deletion and
    /// session close are confirmed first.
    pub(super) fn confirm_delete_selected(&mut self) {
        let Some(process) = self.workspace.selected_process() else {
            return;
        };
        if *process.origin() == ProcessOrigin::Configured {
            // Without an active config there is nothing to edit, so do not offer a
            // confirmation that could only be a no-op.
            let Some(config_path) = self.current_config.clone() else {
                return;
            };
            let message = format!(
                "Delete {} '{}' from muster.yml?",
                process.kind(),
                process.name().as_ref()
            );
            // Bind the confirmation to the immutable identity (spec + occurrence),
            // not the pane id: a watcher reconcile can retire this row and reuse its
            // pane id for a new process before the user confirms.
            let target = ProcessSpecMatcher::of(process);
            // A row already retiring has had its spec removed, so it maps to no live
            // occurrence. Skip it rather than fall back to occurrence 0 and delete a
            // surviving duplicate's spec instead.
            let Some(occurrence) = self.configured_occurrence_of(&target, *process.id()) else {
                self.set_notice(DELETE_ALREADY_RETIRING.to_string());
                return;
            };
            self.overlay = Some(Overlay::ConfirmProcessRemoval {
                message,
                target,
                occurrence,
                config_path,
            });
            return;
        }
        if *process.kind() != ProcessKind::Agent {
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

    /// Whether `pane` is a live row already retiring - an earlier delete removed
    /// its `muster.yml` entry and it is shutting down, so it maps to no surviving
    /// configured occurrence.
    fn pane_is_retiring(&self, pane: PaneId) -> bool {
        self.panes
            .get(&pane)
            .is_some_and(|target| target.config_membership == ConfigMembership::RetireOnExit)
    }

    /// Position of the configured process on `pane` among the still-tracked rows
    /// sharing `target`'s identity, in workspace order - the occurrence that keys
    /// its `muster.yml` spec. Rows already retiring are skipped: their spec is gone
    /// from the config, so counting them would misnumber the occurrence. Returns
    /// `None` when `pane` is itself retiring, since no live spec corresponds to it.
    pub(super) fn configured_occurrence_of(
        &self,
        target: &ProcessSpecMatcher,
        pane: PaneId,
    ) -> Option<usize> {
        self.workspace
            .processes()
            .iter()
            .filter(|candidate| *candidate.origin() == ProcessOrigin::Configured)
            .filter(|candidate| !self.pane_is_retiring(*candidate.id()))
            .filter(|candidate| target.matches_process(candidate))
            .position(|candidate| *candidate.id() == pane)
    }

    /// Deletes the `occurrence`-th configured process matching `target` from
    /// `config_path`, stopping its live row and returning its durable session to
    /// history so a pinned agent stops reappearing on the next reconcile. The row
    /// is resolved by identity, not a stored pane id, so a reconcile that reused
    /// that id between confirmation and now cannot delete the wrong entry.
    pub(super) fn remove_configured_process(
        &mut self,
        target: &ProcessSpecMatcher,
        occurrence: usize,
        config_path: &Path,
    ) {
        // A deferred switch loads the next project from a pane exit event, which can
        // land while this confirmation is open and leaves the old rows selectable
        // until it does. Identity alone does not disambiguate across projects, so a
        // matching spec in the newly loaded `muster.yml` would be deleted instead of
        // the process the user picked. Refuse once the project has moved on.
        if !self
            .current_config
            .as_deref()
            .is_some_and(|current| Self::same_config_location(current, config_path))
        {
            self.set_notice(DELETE_PROJECT_CHANGED.to_string());
            return;
        }
        let mut removed = false;
        let mut updated = None;
        let mut apply = |config: WorkspaceConfig| {
            let (config, hit) = target.without(config, occurrence);
            removed = hit;
            updated = Some(config.clone());
            config
        };
        if self
            .registry
            .update_workspace(config_path, &mut apply)
            .is_err()
        {
            self.set_notice(WORKSPACE_SAVE_ERROR.to_string());
            return;
        }
        // Nothing matched the stored identity: the entry vanished under the open
        // confirmation, so there is nothing to delete. Report it like the sibling
        // no-op branches rather than closing the destructive dialog in silence.
        let Some(config) = updated.filter(|_| removed) else {
            self.set_notice(DELETE_ENTRY_GONE.to_string());
            return;
        };
        self.focus = Focus::Sidebar;
        // Resolve the live row by the same identity, never a stored pane id, and
        // over the same still-tracked rows the occurrence was numbered against so a
        // duplicate already retiring is never miscounted.
        let pane = self
            .workspace
            .processes()
            .iter()
            .filter(|candidate| *candidate.origin() == ProcessOrigin::Configured)
            .filter(|candidate| !self.pane_is_retiring(*candidate.id()))
            .filter(|candidate| target.matches_process(candidate))
            .nth(occurrence)
            .map(|candidate| *candidate.id());
        if let Some(pane) = pane {
            // Mark this exact pane as deleted so reconciliation retires it. With
            // identical duplicate specs a plain reconcile would match the surviving
            // occurrence back to this first row and retire a later duplicate; the
            // mark pins the retirement to the pane the user picked and survives the
            // self-generated watcher reconcile.
            self.deleted_panes.insert(pane);
            // Stop the live row through the same graceful path `s` uses - the
            // configured (or default) stop signal and grace period, then a single
            // escalation - never an immediate force-kill, so a command like a database
            // can clean up. Reuse an active graceful stop rather than re-signaling it:
            // this mirrors `stop_selected`'s `accepts_stop_request` guard, so a process
            // already stopping is not sent a second SIGINT with a reset escalation
            // deadline. Reconcile below still marks it `RetireOnExit`.
            if self.panes.get(&pane).is_some_and(|entry| {
                entry.handle.is_some() && entry.exit_intent.accepts_stop_request()
            }) {
                self.request_graceful_transition(pane, ExitIntent::request_stop, false);
            }
        }
        self.reconcile_config(&config);
        // The deleted row is now retiring, so its key drops out of the live set and
        // recovery closes and un-configures its durable session in one write - the
        // deleted agent's session can never be restored as a disposable one and
        // relaunched.
        self.recover_orphaned_pins();
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
    /// selected one's position among the still-tracked ones, so the persisted
    /// change lands on the row the user picked whatever order a reconcile left the
    /// rows in. A row already retiring has no surviving spec, so it is refused.
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
        // Number the occurrence over the still-tracked rows only, skipping any
        // already retiring: their spec is gone from the config, so counting them
        // would edit a surviving duplicate's spec (or fall past the last one and do
        // nothing). `None` means the selected row is itself retiring, so refuse.
        let Some(occurrence) = self.configured_occurrence_of(&target, pane) else {
            self.set_notice(AUTOSTART_UNTRACKED.to_string());
            return;
        };

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
