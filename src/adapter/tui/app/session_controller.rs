use super::*;
use crate::adapter::process_identity::LocalProcessIdentity;

impl App {
    /// Binds every configured agent that lacks one to a durable session, keyed by
    /// project and a name-based [`ConfiguredAgentKey`] (agent names are unique per
    /// project) so a configured row never binds to a runtime session, reusing an
    /// existing session or creating a fresh one. Once linked, the shared spawn path
    /// resumes the conversation and captures its provider id exactly as a runtime
    /// session does. Idempotent, so it is safe to run after every reconcile and must
    /// run before the initial spawn and restore so an autostarted agent resumes and
    /// its session is never restored twice.
    pub(super) fn link_configured_agent_sessions(&mut self) {
        let Some(project) = self.current_config.clone() else {
            return;
        };
        if self.agent_session_store.is_none() {
            return;
        }
        let mut pending: Vec<(
            PaneId,
            ConfiguredAgentKey,
            ProcessName,
            AgentTool,
            CommandLine,
        )> = Vec::new();
        for process in self.workspace.processes() {
            if *process.kind() != ProcessKind::Agent
                || *process.origin() != ProcessOrigin::Configured
                || process.agent_session_id().is_some()
            {
                continue;
            }
            // A row retiring after its config entry vanished is not part of the
            // current configuration, so it must not be linked to a session.
            if self
                .panes
                .get(process.id())
                .is_some_and(|pane| pane.config_membership == ConfigMembership::RetireOnExit)
            {
                continue;
            }
            let (Some(tool), Some(command)) = (*process.agent_tool(), process.command().clone())
            else {
                continue;
            };
            let Ok(key) = ConfiguredAgentKey::of(process.name()) else {
                continue;
            };
            pending.push((*process.id(), key, process.name().clone(), tool, command));
        }
        for (pane, key, name, tool, command) in pending {
            let Ok(id) = AgentSessionId::generate() else {
                continue;
            };
            // The candidate is inserted only if the store has no session for this
            // project and key; otherwise the store reuses and refreshes the
            // existing record. Get-or-create and refresh happen in one store
            // transaction, so a stale snapshot cannot duplicate or clobber it. A
            // pane left unlinked here is kept from launching by `command_of`.
            let candidate = AgentSession::builder()
                .id(id)
                .name(name)
                .tool(tool)
                .project(project.clone())
                .launch_command(command)
                .configured_key(Some(key))
                .state(AgentSessionState::Closed)
                .build();
            let linked = self
                .agent_session_store
                .as_ref()
                .map(|store| store.link_configured(&candidate));
            match linked {
                Some(Ok(session_id)) => self.workspace.set_agent_session_id(pane, session_id),
                Some(Err(error)) => {
                    self.set_notice(format!("{AGENT_SESSION_STORE_ERROR}: {error}"));
                },
                None => {},
            }
        }
    }

    /// Returns configured sessions with no matching workspace agent to history, so
    /// a pin stranded by a crash between its two durable writes (the session
    /// transferred to configured, but its `muster.yml` entry never written) is
    /// recovered rather than left hidden from both pickers. Scoped to the current
    /// project, whose reconciled agents are the authoritative live set.
    pub(super) fn recover_orphaned_pins(&mut self) {
        let Some(project) = self.current_config.clone() else {
            return;
        };
        let live_keys: Vec<ConfiguredAgentKey> = self
            .workspace
            .processes()
            .iter()
            .filter(|process| {
                *process.kind() == ProcessKind::Agent
                    && *process.origin() == ProcessOrigin::Configured
                    // A row retiring after its config entry vanished (a deleted
                    // agent) is no longer live, so its session is recoverable.
                    && self
                        .panes
                        .get(process.id())
                        .is_none_or(|pane| pane.config_membership != ConfigMembership::RetireOnExit)
            })
            .filter_map(|process| ConfiguredAgentKey::of(process.name()).ok())
            .collect();
        let Some(store) = self.agent_session_store.as_ref() else {
            return;
        };
        // Read the config and retire while holding the workspace lock. After a delete's
        // config write releases that lock, another Muster or CLI instance may re-add the
        // same agent, and the local workspace has not reconciled that yet. Coordinating
        // the config check with config writes - not just the session-store lock - means
        // a re-add cannot land between reading the agent list and retiring: an agent in
        // the locked config is folded in as live so its session is preserved (its
        // conversation association survives the next link), while a genuinely deleted
        // agent is absent from disk and still retires.
        let result = self
            .registry
            .with_workspace_locked(&project, &mut |config| {
                let mut keys = live_keys.clone();
                keys.extend(
                    config
                        .agents()
                        .iter()
                        .filter_map(|spec| ConfiguredAgentKey::of(spec.name()).ok()),
                );
                store.retire_orphaned_configured(&project, &keys)?;
                Ok(())
            });
        if let Err(error) = result {
            self.set_notice(format!("{AGENT_SESSION_STORE_ERROR}: {error}"));
        }
    }

    /// Restores the active project's open durable sessions.
    pub(super) fn restore_open_agent_sessions(&mut self) {
        let Some(project) = self.current_config.clone() else {
            return;
        };
        let sessions: Vec<AgentSession> = match self.agent_sessions() {
            // Configured records are driven by muster.yml, not runtime history, so
            // they are never restored as disposable sessions. Otherwise a removed
            // or renamed configured agent, whose record lingers `Open` with no
            // pane, would be relaunched as a runtime session.
            Ok(sessions) => sessions
                .into_iter()
                .filter(|session| session.configured_key().is_none())
                .collect(),
            Err(error) => {
                self.set_notice(format!("{AGENT_SESSION_STORE_ERROR}: {error}"));
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
                self.set_notice(AGENT_SESSION_NOT_RESUMABLE.to_string());
            }
        }
    }

    /// Returns whether a locally running process still owns the session.
    pub(super) fn session_owner_is_live(session: &AgentSession) -> bool {
        let Some(process_id) = session.owner_process_id() else {
            return false;
        };
        session.owner_process_start_token().is_some_and(|expected| {
            LocalProcessIdentity::start_token(*process_id) == Some(expected)
        })
    }

    /// Whether a pane's linked agent session is currently owned by a live
    /// process. For a pane not running here, that means another Muster instance
    /// sharing the project holds the conversation: launching would lose the
    /// ownership claim and crash the pane only after needlessly starting a
    /// provider, so callers skip the launch. A non-agent, unlinked, or
    /// store-unavailable pane is treated as unowned.
    pub(super) fn agent_session_owned_by_live_process(&self, pane: PaneId) -> bool {
        let Some(session_id) = self
            .workspace
            .process(pane)
            .and_then(|process| process.agent_session_id().clone())
        else {
            return false;
        };
        self.agent_sessions()
            .ok()
            .into_iter()
            .flatten()
            .find(|session| session.id() == &session_id)
            .is_some_and(|session| Self::session_owner_is_live(&session))
    }
}
