use super::*;

impl App {
    /// Handles a key: leader chord, command, or forward to the focused pane.
    pub(super) fn handle_key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            // A release is relayed only when its own press was forwarded, and
            // only to the child that received that press. That drops the release
            // of a leader-consumed command key (whose press never reached a
            // child) and relays a double-leader Ctrl-A's release to the right
            // pane. Releases never touch notices, overlays, or shortcuts.
            self.forward_release(key);
            return;
        }
        // A key press dismisses any transient notice from the previous action
        // and, like herdr, any lingering selection state.
        self.notice = None;
        self.notice_deadline = None;
        self.clear_selection();
        match &self.overlay {
            // Help is a read-only reference; any key closes it.
            Some(Overlay::Help) => {
                self.overlay = None;
                return;
            },
            Some(
                Overlay::ConfirmOverwrite { .. }
                | Overlay::ConfirmRemoval { .. }
                | Overlay::ConfirmSessionClose { .. }
                | Overlay::ConfirmProcessRemoval { .. },
            ) => {
                self.handle_confirm_key(key);
                return;
            },
            Some(Overlay::Form(_)) => {
                self.handle_form_key(key);
                return;
            },
            Some(Overlay::AgentPicker(_)) => {
                self.handle_agent_picker_key(key);
                return;
            },
            Some(Overlay::Switcher(_)) => {
                self.handle_switcher_key(key);
                return;
            },
            None => {},
        }
        match self.focus {
            Focus::Sidebar => self.handle_sidebar_key(key),
            // Enter leader mode only on the chord's own press; a held-chord
            // repeat must not re-enter or, once in leader mode, act as a second
            // press that forwards a literal leader chord and exits.
            Focus::Terminal if is_leader(key) && key.kind == KeyEventKind::Press => {
                self.focus = Focus::Leader;
            },
            Focus::Terminal => self.forward_key(key),
            Focus::Leader if key.kind == KeyEventKind::Repeat => {},
            Focus::Leader => {
                self.focus = Focus::Terminal;
                self.handle_leader_command(key);
            },
        }
    }

    /// Handles a key while the sidebar is focused: direct navigation and actions.
    pub(super) fn handle_sidebar_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') => self.running = false,
            KeyCode::Char('j') | KeyCode::Down => self.sidebar_down(),
            KeyCode::Char('k') | KeyCode::Up => self.sidebar_up(),
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => self.sidebar_select(),
            KeyCode::Char('h') | KeyCode::Left => self.project_cursor = None,
            // On a project row: `d` removes the project; on a process row the
            // process actions apply. Neither leaks across contexts.
            KeyCode::Char('d') if self.project_cursor.is_some() => {
                self.confirm_remove_selected_project();
            },
            KeyCode::Char('d') => self.confirm_delete_selected(),
            KeyCode::Char('u') => self.reopen_last_closed_session(),
            KeyCode::Char('t') if self.project_cursor.is_none() => self.toggle_selected_autostart(),
            KeyCode::Char('s') if self.project_cursor.is_none() => self.toggle_selected(),
            KeyCode::Char('r') if self.project_cursor.is_none() => self.restart_selected(),
            KeyCode::Char('p') if self.project_cursor.is_none() => self.toggle_pause_selected(),
            KeyCode::Char('x') if self.project_cursor.is_none() => self.force_stop_selected(),
            KeyCode::Char('a') => self.open_add_process_form(),
            KeyCode::Char('A') => self.open_agent_picker(),
            KeyCode::Char('!') => {
                self.jump_to_attention();
            },
            KeyCode::Char('e') => self.open_selected_in_editor(),
            KeyCode::Char('n') => self.open_new_project_form(),
            KeyCode::Char('N') => self.toggle_desktop_notifications(),
            KeyCode::Char('o') => self.open_switcher(),
            KeyCode::Char('?') => self.overlay = Some(Overlay::Help),
            _ => {},
        }
    }

    /// Selects the next process awaiting input, cycling from the current
    /// selection so an attention flag can be reached without hunting through
    /// panes. Returns whether one was found; when none is waiting it leaves the
    /// selection put and raises a notice.
    pub(super) fn jump_to_attention(&mut self) -> bool {
        let count = self.workspace.processes().len();
        let start = *self.workspace.selected_index();
        // An empty workspace yields an empty range, so the closure (and its
        // modulo) never runs and the search falls through to the notice below.
        let next = (1..=count)
            .map(|offset| (start + offset) % count)
            .find(|&index| {
                *self.workspace.processes()[index].activity() == ActivityState::AwaitingInput
            });
        match next {
            Some(index) => {
                self.project_cursor = None;
                self.workspace.select_at(index);
                true
            },
            None => {
                self.set_notice(NO_ATTENTION_WAITING.to_string());
                false
            },
        }
    }

    /// Indices into `self.projects` of every project that is not the active one,
    /// in registry order - the collapsed rows below the active project.
    pub(super) fn other_projects(&self) -> Vec<usize> {
        let active = self.current_project_index(&self.projects);
        (0..self.projects.len())
            .filter(|index| Some(*index) != active)
            .collect()
    }

    /// Moves the sidebar selection down: through the active project's processes,
    /// then onto the collapsed project rows, wrapping back to the top.
    pub(super) fn sidebar_down(&mut self) {
        let processes = self.workspace.processes().len();
        let others = self.other_projects().len();
        match self.project_cursor {
            None => {
                let index = *self.workspace.selected_index();
                if index + 1 < processes {
                    self.workspace.select_at(index + 1);
                } else if others > 0 {
                    self.project_cursor = Some(0);
                } else if processes > 0 {
                    self.workspace.select_at(0);
                }
            },
            Some(cursor) if cursor + 1 < others => self.project_cursor = Some(cursor + 1),
            Some(_) => {
                self.project_cursor = None;
                self.workspace.select_at(0);
            },
        }
    }

    /// Moves the sidebar selection up, mirroring [`Self::sidebar_down`].
    pub(super) fn sidebar_up(&mut self) {
        let processes = self.workspace.processes().len();
        let others = self.other_projects().len();
        match self.project_cursor {
            None => {
                let index = *self.workspace.selected_index();
                if index > 0 {
                    self.workspace.select_at(index - 1);
                } else if others > 0 {
                    self.project_cursor = Some(others - 1);
                }
            },
            Some(0) => {
                self.project_cursor = None;
                if processes > 0 {
                    self.workspace.select_at(processes - 1);
                }
            },
            Some(cursor) => self.project_cursor = Some(cursor - 1),
        }
    }

    /// Acts on the sidebar selection: attach to the selected process, or switch
    /// into the selected project.
    pub(super) fn sidebar_select(&mut self) {
        match self.project_cursor {
            Some(cursor) => self.activate_other_project(cursor),
            None => self.focus = Focus::Terminal,
        }
    }

    /// Switches to the `cursor`-th collapsed project, making it active.
    pub(super) fn activate_other_project(&mut self, cursor: usize) {
        let Some(index) = self.other_projects().get(cursor).copied() else {
            return;
        };
        let Some(project) = self.projects.get(index).cloned() else {
            return;
        };
        let config_path = match path::registered_config_path(&project) {
            Ok(config_path) => config_path,
            Err(error) => {
                self.set_notice(error.to_string());
                return;
            },
        };
        match self.registry.workspace(&config_path) {
            Ok(config) => {
                self.project_cursor = None;
                self.begin_switch(config, config_path);
            },
            Err(err) => self.report_project_open_failure(&project, &err),
        }
    }
}
