use super::*;

impl App {
    /// The sidebar display inputs shared by rendering and click hit-testing.
    pub(super) fn sidebar_context(&self) -> (String, Vec<String>, sidebar::SidebarSelection) {
        let active_label = self.active_project_label();
        // Append the folder to a project's label when another project shares its
        // name, so duplicates in the tree are distinguishable.
        let other_projects: Vec<String> =
            self.other_projects()
                .into_iter()
                .filter_map(|index| self.projects.get(index).map(|project| (index, project)))
                .map(|(index, project)| {
                    let name = project.name().as_ref();
                    let duplicated = self.projects.iter().enumerate().any(|(other, candidate)| {
                        other != index && candidate.name().as_ref() == name
                    });
                    if duplicated {
                        format!("{name}  {}", label_from_config(project.config()))
                    } else {
                        name.to_string()
                    }
                })
                .collect();
        let selection = match self.project_cursor {
            Some(cursor) => sidebar::SidebarSelection::Project(cursor),
            None => sidebar::SidebarSelection::Process(*self.workspace.selected_index()),
        };
        (active_label, other_projects, selection)
    }

    /// Draws the whole UI: sidebar, focused terminal, and status bar.
    pub fn render(&self, frame: &mut Frame) {
        let (sidebar_area, main_area, status_area) = areas(frame.area());
        let sidebar_focused = self.focus == Focus::Sidebar;
        let (active_label, other_projects, selection) = self.sidebar_context();
        let sidebar_state = sidebar::SidebarState::builder()
            .workspace(&self.workspace)
            .activity_frame(self.activity_frame)
            .focused(sidebar_focused)
            .active_project(&active_label)
            .other_projects(&other_projects)
            .selection(selection)
            .usage(&self.usage)
            .build();
        sidebar::render(frame, sidebar_area, &sidebar_state);
        let (title, screen) = self.focused_view();
        terminal_pane::render(
            frame,
            main_area,
            &title,
            screen,
            !sidebar_focused,
            self.selection_view,
            self.selection_style,
        );
        if self.workspace.processes().is_empty() {
            empty_state::render(frame, main_area);
        }
        let crashed = self
            .workspace
            .processes()
            .iter()
            .filter(|process| *process.state() == ProcessState::Crashed)
            .count();
        // An error notice floats as a toast when it fits over the pane; when the
        // window is too small for that, it falls back to the always-visible
        // status-bar row so a failure is never invisible.
        let notice_as_toast = self
            .notice
            .as_deref()
            .filter(|notice| toast::region(main_area, notice, ToastTone::Error, 0).is_some());
        let status_notice = self.notice.as_deref().filter(|_| notice_as_toast.is_none());
        status_bar::render(
            frame,
            status_area,
            self.status_context(),
            crashed,
            self.focus == Focus::Leader,
            status_notice,
        );
        if let Some(overlay) = &self.overlay {
            overlay.render(frame);
        }
        // Transient feedback floats above everything, even an open modal, so a
        // notification is never hidden or erased by a modal's clear: it stacks
        // up from the bottom-right of the pane, error notice at the bottom.
        let mut consumed = 0;
        if let Some(notice) = notice_as_toast {
            consumed = toast::render(frame, main_area, notice, ToastTone::Error, consumed);
        }
        if let Some(toast) = &self.toast {
            toast::render(frame, main_area, toast.message(), *toast.tone(), consumed);
        }
    }

    /// The slim hint set the status bar advertises for the current focus and
    /// sidebar selection. The full keymap lives in the `?` overlay.
    pub(super) fn status_context(&self) -> StatusContext {
        if matches!(self.focus, Focus::Terminal | Focus::Leader) {
            return if self.selected_is_agent_session() {
                StatusContext::TerminalAgentSession
            } else {
                StatusContext::Terminal
            };
        }
        match self.project_cursor {
            Some(_) => StatusContext::Project,
            None if self.workspace.is_empty() => StatusContext::Empty,
            None if self.selected_is_agent_session() => StatusContext::AgentSession,
            None => StatusContext::Process,
        }
    }

    /// Whether the selected row is a runtime-managed agent session.
    pub(super) fn selected_is_agent_session(&self) -> bool {
        self.workspace.selected_process().is_some_and(|process| {
            *process.kind() == ProcessKind::Agent && *process.origin() == ProcessOrigin::Session
        })
    }

    /// The (title, emulator) of the currently focused pane. A finished pane still
    /// has its emulator, so its last screen keeps rendering.
    pub(super) fn focused_view(&self) -> (String, Option<&TerminalEmulator>) {
        match self.workspace.selected_process() {
            Some(process) => {
                let emulator = self.panes.get(process.id()).map(|pane| &pane.parser);
                (process.name().as_ref().to_string(), emulator)
            },
            None => (APP_NAME.to_string(), None),
        }
    }
}
