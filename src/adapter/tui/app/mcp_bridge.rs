use super::App;
use crate::{
    adapter::bridge::{
        ControlAction, ControlOutcome, ProcessSummary, SendOutcome, SessionSummary,
        WorkspaceRequest, WorkspaceResponse,
    },
    domain::{agent_session::AgentSessionState, value::PaneId},
};

impl App {
    /// Answers a workspace request from the MCP IPC bridge against live state.
    /// Runs on the event loop (reached via `RuntimeEvent::Command`), so it reads
    /// the roster or writes to a pane's PTY on the same path keyboard input uses -
    /// no render-path locks, no blocking I/O. Domain state is projected onto the
    /// wire here, at the adapter seam.
    pub fn handle_workspace_request(&mut self, request: WorkspaceRequest) -> WorkspaceResponse {
        match request {
            WorkspaceRequest::ListProcesses => WorkspaceResponse::Processes(
                self.workspace
                    .processes()
                    .iter()
                    .map(ProcessSummary::of)
                    .collect(),
            ),
            WorkspaceRequest::PaneOutput { pane } => {
                WorkspaceResponse::PaneOutput(self.pane_output(PaneId::new(pane)))
            },
            WorkspaceRequest::SendInput { pane, data } => {
                WorkspaceResponse::Sent(self.send_pane_input(PaneId::new(pane), data.as_bytes()))
            },
            WorkspaceRequest::ControlProcess { pane, action } => {
                WorkspaceResponse::Controlled(self.control_process(PaneId::new(pane), action))
            },
            WorkspaceRequest::ListSessions => self.session_response(),
        }
    }

    /// Drives `pane`'s lifecycle through the very methods the keybindings use, so
    /// an agent's request honours the same session relinking, graceful-stop
    /// signalling, and exit-intent gating the human's keystroke does.
    fn control_process(&mut self, pane: PaneId, action: ControlAction) -> ControlOutcome {
        if self.workspace.process(pane).is_none() {
            return ControlOutcome::Unknown;
        }
        match action {
            ControlAction::Start => self.start_pane(pane),
            ControlAction::Stop => self.stop_pane(pane),
            ControlAction::Restart => self.restart_pane(pane),
        }
        ControlOutcome::Requested
    }

    /// This project's durable agent sessions. A store failure answers
    /// `Unavailable` rather than an empty list, so "could not read" is never
    /// mistaken for "no sessions".
    fn session_response(&self) -> WorkspaceResponse {
        let (Some(store), Some(project)) = (
            self.agent_session_store.as_ref(),
            self.current_config.as_deref(),
        ) else {
            return WorkspaceResponse::Sessions(Vec::new());
        };
        match store.sessions() {
            Ok(sessions) => WorkspaceResponse::Sessions(
                sessions
                    .iter()
                    .filter(|session| session.project() == project)
                    .map(|session| {
                        SessionSummary::builder()
                            .name(session.name().as_ref().to_string())
                            .tool(session.tool().to_string())
                            .state(
                                match session.state() {
                                    AgentSessionState::Pending => "pending",
                                    AgentSessionState::Open => "open",
                                    AgentSessionState::Closed => "closed",
                                }
                                .to_string(),
                            )
                            .resumable(session.native_id().is_some())
                            .build()
                    })
                    .collect(),
            ),
            Err(_) => WorkspaceResponse::Unavailable,
        }
    }

    /// The latest on-screen text of `pane`. A live pane returns its screen; a
    /// process that exists but has not started its terminal yet returns empty
    /// (not "unknown"); only a truly unknown id returns `None`.
    fn pane_output(&self, pane: PaneId) -> Option<String> {
        if let Some(live) = self.panes.get(&pane) {
            Some(live.parser.screen_text())
        } else if self.workspace.process(pane).is_some() {
            Some(String::new())
        } else {
            None
        }
    }

    /// Writes `data` to the terminal of the process owning `pane`, on the same
    /// path keyboard input takes. Distinguishes delivery from a stopped process
    /// and from an unknown id.
    fn send_pane_input(&mut self, pane: PaneId, data: &[u8]) -> SendOutcome {
        if let Some(live) = self.panes.get_mut(&pane) {
            // A live handle whose write succeeds delivered the input; a missing
            // handle or a failed write means the process is stopped.
            if let Some(handle) = live.handle.as_mut() {
                if handle.write_input(data).is_ok() {
                    SendOutcome::Delivered
                } else {
                    SendOutcome::NotRunning
                }
            } else {
                SendOutcome::NotRunning
            }
        } else if self.workspace.process(pane).is_some() {
            SendOutcome::NotRunning
        } else {
            SendOutcome::Unknown
        }
    }
}
