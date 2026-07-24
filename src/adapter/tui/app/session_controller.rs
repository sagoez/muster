use super::*;
use crate::adapter::process_identity::LocalProcessIdentity;

impl App {
    /// Restores the active project's open durable sessions.
    pub(super) fn restore_open_agent_sessions(&mut self) {
        let Some(project) = self.current_config.clone() else {
            return;
        };
        let sessions = match self.agent_sessions() {
            Ok(sessions) => sessions,
            Err(error) => {
                self.notice = Some(format!("{AGENT_SESSION_STORE_ERROR}: {error}"));
                return;
            },
        };
        let existing: HashSet<AgentSessionId> = self
            .workspace
            .processes()
            .iter()
            .filter_map(|process| process.agent_session_id().clone())
            .collect();
        for restore in SessionRestorer::for_project(
            sessions,
            &project,
            &existing,
            Self::same_config_location,
            Self::session_owner_is_live,
        ) {
            if let Some(command) = restore.command().clone() {
                self.insert_agent_session(
                    restore.session(),
                    AgentSessionActivation::StartDetached(command),
                );
            } else {
                self.insert_agent_session(restore.session(), AgentSessionActivation::Stopped);
                self.notice = Some(AGENT_SESSION_NOT_RESUMABLE.to_string());
            }
        }
    }

    /// Returns whether a locally running process still owns the session.
    fn session_owner_is_live(session: &AgentSession) -> bool {
        let Some(process_id) = session.owner_process_id() else {
            return false;
        };
        session.owner_process_start_token().is_some_and(|expected| {
            LocalProcessIdentity::start_token(*process_id) == Some(expected)
        })
    }
}
