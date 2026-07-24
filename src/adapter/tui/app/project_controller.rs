use super::*;

impl App {
    /// Reloads the cached registered-project list shown in the sidebar tree.
    pub(super) fn refresh_projects(&mut self) {
        let mut projects = self.registry.projects().unwrap_or_default();
        let launched = self.launched_config.clone();
        let registered = projects
            .iter()
            .any(|project| Self::same_config_location(project.config(), &launched));
        self.launched_project_membership = LaunchedProjectMembership::Registered;
        if !registered && let Ok(name) = ProjectName::try_new(label_from_config(&launched)) {
            projects.insert(0, Project::builder().name(name).config(launched).build());
            self.launched_project_membership = LaunchedProjectMembership::Synthetic;
        }
        self.projects = projects;
    }

    /// Re-points the config watcher at the active project's config.
    pub(super) fn rewatch_config(&mut self) {
        if let (Some(watcher), Some(config)) = (self.watcher.as_mut(), self.current_config.as_ref())
        {
            watcher.watch(config);
        }
    }

    /// Reconciles the active workspace when its watched configuration changes.
    pub fn handle_config_changed(&mut self, path: PathBuf) {
        let Some(current) = self.current_config.clone() else {
            return;
        };
        if path::normalize(&current) != path::normalize(&path) {
            return;
        }
        let Ok(config) = self.registry.workspace(&current) else {
            return;
        };
        self.reconcile_config(&config);
    }

    /// Applies changed disk configuration without bouncing surviving processes.
    pub(super) fn reconcile_config(&mut self, config: &WorkspaceConfig) {
        let reconciliation = Reconciliation::apply(&self.workspace, config, |pane| {
            self.panes
                .get(&pane)
                .is_some_and(|target| target.handle.is_some())
        });
        let tracked = reconciliation.tracked().clone();
        let retiring = reconciliation.retiring().clone();
        let removed = reconciliation.removed().clone();
        for pane in tracked {
            if let Some(target) = self.panes.get_mut(&pane) {
                target.config_membership = ConfigMembership::Tracked;
            }
        }
        for pane in retiring {
            if let Some(target) = self.panes.get_mut(&pane) {
                target.config_membership = ConfigMembership::RetireOnExit;
            }
        }
        for pane in removed {
            self.panes.remove(&pane);
            self.generations.remove(&pane);
            self.restart_attempts.remove(&pane);
        }
        self.workspace = reconciliation.into_workspace();
    }

    /// Switches to the project selected in the project switcher.
    pub(super) fn switch_to(&mut self, index: usize) {
        let Some(project) = self
            .switcher()
            .and_then(|switcher| switcher.projects.get(index))
            .cloned()
        else {
            return;
        };
        let config_path = match path::registered_config_path(&project) {
            Ok(config_path) => config_path,
            Err(error) => {
                if let Some(switcher) = self.switcher_mut() {
                    switcher.selected = index;
                    switcher.error = Some(error.to_string());
                }
                return;
            },
        };
        match self.registry.workspace(&config_path) {
            Ok(config) => {
                self.begin_switch(config, config_path);
                self.overlay = None;
            },
            Err(err) if self.registry.workspace_exists(&config_path) => {
                if let Some(switcher) = self.switcher_mut() {
                    switcher.selected = index;
                    switcher.error = Some(err.to_string());
                }
            },
            Err(err) => self.report_project_open_failure(&project, &err),
        }
    }

    /// Stops live panes and defers loading until their exit events arrive.
    pub(super) fn begin_switch(&mut self, config: WorkspaceConfig, config_path: PathBuf) {
        self.pending_session_reopens.clear();
        let live: HashSet<PaneId> = self
            .panes
            .iter()
            .filter(|(_, pane)| pane.handle.is_some())
            .map(|(id, _)| *id)
            .collect();
        if live.is_empty() {
            self.load_project(config, config_path);
            return;
        }
        for pane in &live {
            if let Some(target) = self.panes.get_mut(pane)
                && let Some(handle) = target.handle.as_mut()
            {
                target.exit_intent = target.exit_intent.request_stop();
                if handle.kill().is_ok() {
                    target.exit_intent = target.exit_intent.stop_delivered();
                }
            }
        }
        self.pending_switch = Some(PendingSwitch {
            config,
            config_path,
            waiting: live,
        });
    }

    /// Advances a deferred switch after one old pane exits.
    pub(super) fn advance_pending_switch(&mut self, pane: PaneId) {
        let ready = match self.pending_switch.as_mut() {
            Some(pending) => {
                pending.waiting.remove(&pane);
                pending.waiting.is_empty()
            },
            None => return,
        };
        if ready && let Some(pending) = self.pending_switch.take() {
            let config = self
                .registry
                .workspace(&pending.config_path)
                .unwrap_or(pending.config);
            self.load_project(config, pending.config_path);
        }
    }

    /// Replaces the current workspace and starts its configured processes.
    pub(super) fn load_project(&mut self, config: WorkspaceConfig, config_path: PathBuf) {
        for pane in self.panes.values_mut() {
            if let Some(handle) = pane.handle.as_mut() {
                let _ = handle.kill();
            }
        }
        self.panes.clear();
        self.restart_attempts.clear();
        self.pending_session_reopens.clear();
        self.workspace = Workspace::builder()
            .processes(config.to_processes())
            .build();
        self.current_config = Some(path::absolutize(&config_path));
        self.focus = Focus::Sidebar;
        self.rewatch_config();
        self.start();
    }

    /// Compares stored config locations without resolving symlink aliases.
    pub(super) fn same_config_location(left: &Path, right: &Path) -> bool {
        let left = path::expand_home(left);
        let right = path::expand_home(right);
        left.is_absolute() == right.is_absolute() && left == right
    }

    /// Returns the active project index by its stored configuration location.
    pub(super) fn current_project_index(&self, projects: &[Project]) -> Option<usize> {
        let current = self.current_config.as_deref()?;
        projects
            .iter()
            .position(|project| Self::same_config_location(project.config(), current))
    }
}
