use std::{collections::HashSet, path::PathBuf};

use fake::{Fake, faker::name::en::FirstName};

use super::{
    config_membership::ConfigMembership,
    entity::Supervisor,
    launch_intent::LaunchIntent,
    lifecycle::STOP_DELIVERY_FAILED_NOTICE,
    pending_intent::PendingIntent,
    reopen_outcome::ReopenOutcome,
    session_row::SessionRow,
    supervisor_error::{AGENT_SESSION_NOT_RESUMABLE, AGENT_SESSION_STORE_ERROR, SupervisorError},
};
use crate::{
    adapter::{path, process_identity::LocalProcessIdentity, service::NoticeLevel},
    application::SessionRestorer,
    domain::{
        agent_session::{
            AgentProcessId, AgentSession, AgentSessionId, AgentSessionState, ConfiguredAgentKey,
            LaunchToken,
        },
        config::ConfigError,
        port::AgentSessionStore,
        process::{AgentProtocol, AgentTool, Process, ProcessKind, ProcessOrigin, ProcessState},
        value::{CommandLine, PaneId, ProcessName},
    },
};

/// Shown when a close names a process that is not a runtime agent session.
const NOT_AN_AGENT_SESSION: &str = "only an agent session can be closed";
/// Attempts to avoid a generated display-name collision in the workspace.
const GENERATED_NAME_ATTEMPTS: usize = 8;
/// Leading characters of a session id a fallback session name carries.
const SESSION_NAME_ID_CHARS: usize = 8;

/// Command details required to spawn one workspace pane.
type SpawnDetails = (Option<CommandLine>, Option<PathBuf>);

impl Supervisor {
    /// Opens a new `tool` conversation running `command`, or the preset's
    /// default when absent, named `name` or a generated name, resuming through
    /// the `resume` template when given, and starts it. Returns the session's
    /// name.
    ///
    /// # Errors
    /// Returns a [`SupervisorError`] when there is no command, the template is
    /// unsafe, or the session cannot be recorded.
    pub fn new_agent_session(
        &mut self,
        tool: AgentTool,
        name: Option<ProcessName>,
        command: Option<CommandLine>,
        resume: Option<CommandLine>,
    ) -> Result<ProcessName, SupervisorError> {
        let row = self.create_agent_session(tool, name, command, resume)?;
        self.workspace
            .process(row.pane())
            .map(|process| process.name().clone())
            .ok_or(SupervisorError::SessionIdentityUnavailable)
    }

    /// Creates, records and starts a new agent conversation. `command`
    /// overrides the provider's default and `resume_command` is a custom
    /// resume template.
    ///
    /// # Errors
    /// Returns a [`SupervisorError`] when the template is unsafe, there is no
    /// command, or the session cannot be recorded.
    pub(crate) fn create_agent_session(
        &mut self,
        tool: AgentTool,
        name: Option<ProcessName>,
        command: Option<CommandLine>,
        resume_command: Option<CommandLine>,
    ) -> Result<SessionRow, SupervisorError> {
        if resume_command
            .as_ref()
            .is_some_and(|template| !AgentSession::resume_template_is_valid(template))
        {
            return Err(SupervisorError::ResumeTemplateInvalid);
        }
        let command = command
            .or_else(|| {
                tool.default_command()
                    .and_then(|command| CommandLine::try_new(command).ok())
            })
            .ok_or(SupervisorError::CommandRequired)?;
        let id =
            AgentSessionId::generate().map_err(|_| SupervisorError::SessionIdentityUnavailable)?;
        if let Some(name) = name.as_ref().filter(|name| self.name_in_use(name)) {
            return Err(SupervisorError::NameTaken(name.clone()));
        }
        let name = name
            .or_else(|| self.generated_agent_name(tool, &id))
            .ok_or(SupervisorError::SessionIdentityUnavailable)?;
        let launch_command = tool
            .new_session_command(&command, &id)
            .ok_or(SupervisorError::CommandRequired)?;
        let session = AgentSession::builder()
            .id(id)
            .name(name)
            .tool(tool)
            .project(self.project.as_ref().clone())
            .launch_command(command)
            .resume_command(resume_command)
            .state(AgentSessionState::Pending)
            .build();
        if let Some(store) = &self.agent_session_store {
            store
                .upsert(&session)
                .map_err(SupervisorError::SessionStore)?;
        }
        Ok(self.insert_agent_session(&session, Some(launch_command)))
    }

    /// A friendly non-identifying session name, avoiding names already in use
    /// when it can and falling back to the provider plus the id's prefix.
    fn generated_agent_name(&self, tool: AgentTool, id: &AgentSessionId) -> Option<ProcessName> {
        self.generated_unused_name().or_else(|| {
            let suffix: String = id.as_ref().chars().take(SESSION_NAME_ID_CHARS).collect();
            ProcessName::try_new(format!("{tool} {suffix}")).ok()
        })
    }

    /// A generated friendly name no current process uses, if a few attempts
    /// find one.
    fn generated_unused_name(&self) -> Option<ProcessName> {
        (0..GENERATED_NAME_ATTEMPTS).find_map(|_| {
            let generated: String = FirstName().fake();
            ProcessName::try_new(generated)
                .ok()
                .filter(|name| !self.name_in_use(name))
        })
    }

    /// Whether any current process, of any kind, already uses `name`, however
    /// its accents are encoded.
    fn name_in_use(&self, name: &ProcessName) -> bool {
        self.name_in_use_except(name, |_| false)
    }

    /// Whether a current process other than those `own` claims already uses
    /// `name`, however its accents are encoded.
    fn name_in_use_except(&self, name: &ProcessName, own: impl Fn(&Process) -> bool) -> bool {
        let key = name.key();
        self.workspace
            .processes()
            .iter()
            .any(|process| process.name().key() == key && !own(process))
    }

    /// Restores the project's open runtime sessions as rows, started with their
    /// resume command, or stopped with a notice when they cannot resume.
    pub(super) fn restore_open_agent_sessions(&mut self) {
        let sessions: Vec<AgentSession> = match self.agent_sessions() {
            // A configured record follows muster.yml, so a removed or renamed
            // configured agent whose record lingers open is never relaunched as
            // a runtime session.
            Ok(sessions) => sessions
                .into_iter()
                .filter(|session| session.configured_key().is_none())
                .collect(),
            Err(error) => {
                self.notify(
                    NoticeLevel::Error,
                    None,
                    format!("{AGENT_SESSION_STORE_ERROR}: {error}"),
                );
                return;
            },
        };
        let existing: HashSet<AgentSessionId> = self
            .workspace
            .processes()
            .iter()
            .filter_map(|process| process.agent_session_id().clone())
            .collect();
        let restores = SessionRestorer::for_project(
            sessions,
            self.project.as_ref(),
            &existing,
            path::same_config_location,
            Self::session_owner_is_live,
        );
        for restore in restores {
            let command = restore.command().clone();
            let resumable = command.is_some();
            let row = self.insert_agent_session(restore.session(), command);
            if !resumable {
                self.notify(
                    NoticeLevel::Error,
                    Some(row.pane()),
                    AGENT_SESSION_NOT_RESUMABLE,
                );
            }
        }
    }

    /// Represents `session` as a row, reusing its row when it has one, and
    /// starts it with `start` unless it is already running.
    fn insert_agent_session(
        &mut self,
        session: &AgentSession,
        start: Option<CommandLine>,
    ) -> SessionRow {
        let existing = self
            .workspace
            .processes()
            .iter()
            .find(|process| process.agent_session_id().as_ref() == Some(session.id()))
            .map(|process| *process.id());
        if let Some(pane) = existing {
            if let Some(command) = start
                && !self.is_live(pane)
            {
                self.spawn(pane, Some(command), session.working_dir().clone());
            }
            return SessionRow::Existing(pane);
        }
        let pane = self.next_pane_id();
        let process = Process::builder()
            .id(pane)
            .name(session.name().clone())
            .kind(ProcessKind::Agent)
            .agent_tool(Some(*session.tool()))
            .agent_session_id(Some(session.id().clone()))
            .origin(ProcessOrigin::Session)
            .command(start.clone())
            .working_dir(session.working_dir().clone())
            .autostart(start.is_some())
            .build();
        self.workspace.insert_in_section(process);
        if let Some(command) = start {
            self.spawn(pane, Some(command), session.working_dir().clone());
        }
        SessionRow::Inserted(pane)
    }

    /// A pane id no configured or runtime process uses.
    fn next_pane_id(&self) -> PaneId {
        let next = self
            .workspace
            .processes()
            .iter()
            .map(|process| process.id().into_inner())
            .max()
            .map_or(0, |pane| pane + 1);
        PaneId::new(next)
    }

    /// Reopens the recorded session `id` with its provider-native resume
    /// command. A session whose previous row is still shutting down reopens
    /// once that row has exited. Every reopen path, queued ones included,
    /// comes through here.
    ///
    /// # Errors
    /// Returns a [`SupervisorError`] when the session is unknown, cannot
    /// resume, another process took its name meanwhile, or its state cannot
    /// be recorded.
    pub(crate) fn reopen_agent_session(
        &mut self,
        id: &AgentSessionId,
    ) -> Result<ReopenOutcome, SupervisorError> {
        let session = self
            .agent_sessions()
            .map_err(SupervisorError::SessionStore)?
            .into_iter()
            .find(|session| session.id() == id)
            .ok_or(SupervisorError::NoRecentSession)?;
        let command = session
            .reopen_command()
            .ok_or(SupervisorError::NotResumable)?;
        // Its own row, still closing, may hold the name.
        if self.name_in_use_except(session.name(), |process| {
            process.agent_session_id().as_ref() == Some(session.id())
        }) {
            return Err(SupervisorError::NameTaken(session.name().clone()));
        }
        if let Some(store) = &self.agent_session_store {
            store
                .set_state(session.id(), AgentSessionState::Open)
                .map_err(SupervisorError::SessionStore)?;
        }
        let closing = self
            .workspace
            .processes()
            .iter()
            .filter(|process| process.agent_session_id().as_ref() == Some(session.id()))
            .map(|process| *process.id())
            .find(|pane| {
                self.panes.get(pane).is_some_and(|target| {
                    target.handle.is_some()
                        && target.config_membership == ConfigMembership::RetireOnExit
                })
            });
        if let Some(pane) = closing {
            self.pending_session_reopens
                .insert(pane, session.id().clone());
            return Ok(ReopenOutcome::Queued);
        }
        Ok(ReopenOutcome::Opened(
            self.insert_agent_session(&session, Some(command)),
        ))
    }

    /// Reopens a session queued behind its previous row's exit, reporting a
    /// failure as a notice. Returns the row it landed on.
    pub(super) fn reopen_queued(&mut self, id: &AgentSessionId) -> Option<SessionRow> {
        match self.reopen_agent_session(id) {
            Ok(ReopenOutcome::Opened(row)) => Some(row),
            Ok(ReopenOutcome::Queued) => None,
            Err(error) => {
                self.notify(NoticeLevel::Error, None, error.to_string());
                None
            },
        }
    }

    /// The newest closed, resumable runtime session of this project.
    ///
    /// # Errors
    /// Returns a [`SupervisorError`] when history cannot be read, there is no
    /// closed session, or none of them can resume.
    pub(crate) fn last_closed_session(&self) -> Result<AgentSession, SupervisorError> {
        let sessions = self
            .agent_sessions()
            .map_err(SupervisorError::SessionStore)?;
        let project = self.project.as_ref();
        // A configured record follows muster.yml and is never reopened as a
        // runtime session.
        let belongs_to_project = |session: &&AgentSession| {
            session.configured_key().is_none()
                && *session.state() == AgentSessionState::Closed
                && path::same_config_location(session.project(), project)
        };
        let has_closed = sessions.iter().any(|session| belongs_to_project(&session));
        sessions
            .iter()
            .rev()
            .filter(belongs_to_project)
            .find(|session| session.reopen_command().is_some())
            .cloned()
            .ok_or(if has_closed {
                SupervisorError::NotResumable
            } else {
                SupervisorError::NoRecentSession
            })
    }

    /// Reopens the newest closed resumable session of this project. Returns
    /// its name and whether it runs now or is queued behind its previous row's
    /// exit.
    ///
    /// # Errors
    /// Returns a [`SupervisorError`] when there is no such session or it
    /// cannot be reopened.
    pub fn reopen_last_closed(&mut self) -> Result<(ProcessName, ReopenOutcome), SupervisorError> {
        let session = self.last_closed_session()?;
        let outcome = self.reopen_agent_session(session.id())?;
        Ok((session.name().clone(), outcome))
    }

    /// Closes the runtime agent session named `name`, reporting a name that
    /// is not one as a notice.
    pub fn close_agent_session(&mut self, name: &ProcessName) {
        let Some(pane) = self.resolve(name) else {
            return;
        };
        let is_session = self.workspace.process(pane).is_some_and(|process| {
            *process.kind() == ProcessKind::Agent && *process.origin() == ProcessOrigin::Session
        });
        if is_session {
            self.close_agent_session_pane(pane);
        } else {
            self.notify(NoticeLevel::Error, Some(pane), NOT_AN_AGENT_SESSION);
        }
    }

    /// Force-kills the agent session on `pane` and records it closed only once
    /// the kill was delivered. Its row retires on exit, or at once when it was
    /// not running. A session still starting is recorded closed: a launch's
    /// row retires at once and its arriving process is discarded, so nothing
    /// reopens it, while an adoption keeps the kill and the retirement, so a
    /// child it could not reach is still killed and never left running
    /// without a row. Failures are notices.
    pub(crate) fn close_agent_session_pane(&mut self, pane: PaneId) {
        let session_id = self
            .workspace
            .process(pane)
            .and_then(|process| process.agent_session_id().clone());
        if self.is_starting(pane) {
            if !self.persist_agent_session_state(
                pane,
                session_id.as_ref(),
                AgentSessionState::Closed,
            ) {
                return;
            }
            if self.is_spawning(pane) {
                self.retire_pane(pane);
            } else {
                self.pending_session_reopens.remove(&pane);
                self.record_intent(pane, PendingIntent::Kill);
                self.set_membership(pane, ConfigMembership::RetireOnExit);
            }
            return;
        }
        if !self.is_live(pane) {
            if self.persist_agent_session_state(
                pane,
                session_id.as_ref(),
                AgentSessionState::Closed,
            ) {
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
            self.notify(NoticeLevel::Error, Some(pane), STOP_DELIVERY_FAILED_NOTICE);
            return;
        }
        self.workspace.set_state(pane, ProcessState::Stopping);
        if !self.persist_agent_session_state(pane, session_id.as_ref(), AgentSessionState::Closed) {
            return;
        }
        self.pending_session_reopens.remove(&pane);
        self.set_membership(pane, ConfigMembership::RetireOnExit);
    }

    /// Records `pane`'s session as `state`, reporting a store failure as a
    /// notice. Returns whether the transition may proceed.
    fn persist_agent_session_state(
        &self,
        pane: PaneId,
        session_id: Option<&AgentSessionId>,
        state: AgentSessionState,
    ) -> bool {
        let (Some(store), Some(session_id)) = (self.agent_session_store.as_ref(), session_id)
        else {
            return true;
        };
        match store.set_state(session_id, state) {
            Ok(()) => true,
            Err(error) => {
                self.report_session_store_error(pane, error);
                false
            },
        }
    }

    /// Returns the command and working directory for a pane's next spawn.
    /// `intent` selects resume semantics for a linked agent session: an
    /// automatic launch restores (never silently starting a fresh conversation
    /// for an owned-but-uncaptured reporter session), a user launch reopens.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when durable session history cannot be read.
    pub(super) fn command_of(
        &self,
        pane: PaneId,
        intent: LaunchIntent,
    ) -> Result<Option<SpawnDetails>, ConfigError> {
        let Some(process) = self.workspace.process(pane) else {
            return Ok(None);
        };
        let command = if let Some(session_id) = process.agent_session_id() {
            let Some(session) = self
                .agent_sessions()?
                .into_iter()
                .find(|session| session.id() == session_id)
            else {
                return Ok(None);
            };
            let resolved = match intent {
                LaunchIntent::Automatic => session.restore_command(),
                LaunchIntent::UserInitiated => session.reopen_command(),
            };
            match resolved {
                Some(command) => Some(command),
                // A custom configured agent has no built-in resume flag, so it
                // relaunches its raw command and resumes through the injected
                // session environment. A provider agent that resolves to no
                // command (an owned-but-uncaptured reporter under restore
                // semantics) is surfaced as stopped rather than silently
                // relaunched into a fresh conversation; a runtime session has no
                // configured command to fall back on.
                None if *process.origin() == ProcessOrigin::Configured
                    && *process.agent_tool() == Some(AgentTool::Custom) =>
                {
                    process.command().clone()
                },
                None => return Ok(None),
            }
        } else if self.agent_session_store.is_some()
            && *process.kind() == ProcessKind::Agent
            && *process.origin() == ProcessOrigin::Configured
        {
            // A configured agent with no linked session (a store is present but
            // the link never persisted) must not launch its raw command into a
            // fresh untracked conversation; surface it as unlaunchable so the
            // user retries once the store is writable again.
            return Ok(None);
        } else {
            process.command().clone()
        };
        Ok(Some((command, process.working_dir().clone())))
    }

    /// Loads durable session history, returning an empty list in compositions
    /// that deliberately omit the store.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the store cannot be read.
    pub(crate) fn agent_sessions(&self) -> Result<Vec<AgentSession>, ConfigError> {
        self.agent_session_store
            .as_ref()
            .map_or_else(|| Ok(Vec::new()), |store| store.sessions())
    }

    /// Binds every configured agent that lacks one to a durable session, keyed
    /// by project and a name-based [`ConfiguredAgentKey`] (agent names are
    /// unique per project) so a configured row never binds to a runtime
    /// session, reusing an existing session or creating a fresh one.
    /// Idempotent, so it is safe to run after every reconcile, and it must run
    /// before the initial spawn so an autostarted agent resumes.
    pub(super) fn link_configured_agent_sessions(&mut self) {
        if self.agent_session_store.is_none() {
            return;
        }
        let project = self.project.as_ref().clone();
        let pending: Vec<(
            PaneId,
            ConfiguredAgentKey,
            ProcessName,
            AgentTool,
            CommandLine,
        )> = self
            .workspace
            .processes()
            .iter()
            .filter(|process| {
                *process.kind() == ProcessKind::Agent
                    && *process.origin() == ProcessOrigin::Configured
                    && process.agent_session_id().is_none()
            })
            // A row retiring after its config entry vanished is not part of the
            // current configuration, so it must not be linked to a session.
            .filter(|process| {
                self.panes
                    .get(process.id())
                    .is_none_or(|pane| pane.config_membership != ConfigMembership::RetireOnExit)
            })
            .filter_map(|process| {
                let (Some(tool), Some(command)) =
                    (*process.agent_tool(), process.command().clone())
                else {
                    return None;
                };
                let key = ConfiguredAgentKey::of(process.name()).ok()?;
                Some((*process.id(), key, process.name().clone(), tool, command))
            })
            .collect();
        for (pane, key, name, tool, command) in pending {
            let Ok(id) = AgentSessionId::generate() else {
                continue;
            };
            // The candidate is inserted only if the store has no session for this
            // project and key; otherwise the store reuses and refreshes the
            // existing record in one transaction. A pane left unlinked here is
            // kept from launching by `command_of`.
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
                Some(Err(error)) => self.report_session_store_error(pane, error),
                None => {},
            }
        }
    }

    /// Returns configured sessions with no matching workspace agent to history,
    /// so an agent removed from `muster.yml` leaves a resumable session rather
    /// than a hidden one.
    pub(super) fn recover_orphaned_configured_sessions(&mut self) {
        let project = self.project.as_ref().clone();
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
        // Read the config and retire while holding the workspace lock, so another
        // instance re-adding the same agent between reading the agent list and
        // retiring keeps its session: an agent in the locked config is folded in
        // as live, while a genuinely deleted agent is absent and still retires.
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
            self.notify(
                NoticeLevel::Error,
                None,
                format!("{AGENT_SESSION_STORE_ERROR}: {error}"),
            );
        }
    }

    /// Retries persisting a configured agent's durable session before a manual
    /// launch, so a start after a transient store failure links and resumes
    /// rather than staying blocked. A no-op for an already-linked pane, a
    /// runtime session, or a non-agent process.
    pub(super) fn relink_unlinked_agent(&mut self, pane: PaneId) {
        let needs_link = self.agent_session_store.is_some()
            && self.workspace.process(pane).is_some_and(|process| {
                *process.kind() == ProcessKind::Agent
                    && *process.origin() == ProcessOrigin::Configured
                    && process.agent_session_id().is_none()
            });
        if needs_link {
            self.link_configured_agent_sessions();
        }
    }

    /// Returns whether a locally running process still owns the session.
    pub(crate) fn session_owner_is_live(session: &AgentSession) -> bool {
        let Some(process_id) = session.owner_process_id() else {
            return false;
        };
        session.owner_process_start_token().is_some_and(|expected| {
            LocalProcessIdentity::start_token(*process_id) == Some(expected)
        })
    }

    /// Whether a pane's linked agent session is currently owned by a live
    /// process. For a pane not running here, that means another muster instance
    /// sharing the project holds the conversation. A non-agent, unlinked, or
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

    /// Synchronously records the freshly spawned launcher as the session
    /// owner. The launch child execs the provider, so the PTY child pid is the
    /// durable owner and this claim composes with the child's own re-claim of
    /// the same pid.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the claim cannot be persisted or another
    /// live launcher already owns the session.
    pub(super) fn claim_spawned_owner(
        store: &dyn AgentSessionStore,
        session_id: &AgentSessionId,
        process_id: Option<u32>,
        launch_token: Option<LaunchToken>,
    ) -> Result<(), ConfigError> {
        let Some(process_id) = process_id.and_then(|pid| AgentProcessId::try_new(pid).ok()) else {
            return Ok(());
        };
        store.set_owner_process_id(
            session_id,
            process_id,
            LocalProcessIdentity::start_token(process_id),
            launch_token,
        )
    }

    /// Reports a session-store failure about `pane` without confusing it with an
    /// absent identity.
    pub(super) fn report_session_store_error(&self, pane: PaneId, error: ConfigError) {
        self.notify(
            NoticeLevel::Error,
            Some(pane),
            format!("{AGENT_SESSION_STORE_ERROR}: {error}"),
        );
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        ffi::{OsStr, OsString},
        path::{Path, PathBuf},
        rc::Rc,
        sync::{Arc, atomic::Ordering},
    };

    use super::*;
    use crate::{
        adapter::service::supervisor::{
            entity::UNKNOWN_PROCESS_NOTICE, output_effect::OutputEffect, testing::*,
        },
        constants::MUSTER_AGENT_SESSION_ENV,
        domain::{
            agent_session::NativeSessionId,
            config::{ProcessSpec, WorkspaceConfig},
            port::ProjectRegistry,
            process::{ExitIntent, ProcessKind},
            project::Project,
            pty::{ExitOutcome, ProcessOutput},
        },
    };

    /// `name` as a process name.
    fn name(name: &str) -> ProcessName {
        ProcessName::try_new(name).unwrap()
    }

    /// A configured agent named `agent` running `command` on pane `id`.
    fn configured_agent(id: u64, agent: &str, command: &str) -> Process {
        ProcessSpec::builder()
            .name(name(agent))
            .command(Some(CommandLine::try_new(command).unwrap()))
            .autostart(true)
            .build()
            .to_process(PaneId::new(id), ProcessKind::Agent)
    }

    /// A supervisor over `processes` with durable history held by `sessions`,
    /// nothing started yet.
    fn with_sessions(processes: Vec<Process>, sessions: &SessionRecorder) -> Harness {
        let mut harness = Harness::new(Box::new(FakeRunner), processes);
        harness
            .supervisor
            .set_agent_session_store(Box::new(FakeAgentSessionStore {
                recorder: sessions.clone(),
            }));
        harness
    }

    /// The closed configured record `agent` of the test project would link to.
    fn configured_record(agent: &str, tool: AgentTool, command: &str) -> AgentSession {
        configured_record_with(
            agent,
            tool,
            command,
            AgentSessionId::generate().unwrap(),
            None,
            AgentSessionState::Closed,
        )
    }

    /// A configured record for `agent` with the given id, captured identity
    /// and state.
    fn configured_record_with(
        agent: &str,
        tool: AgentTool,
        command: &str,
        id: AgentSessionId,
        native_id: Option<&str>,
        state: AgentSessionState,
    ) -> AgentSession {
        AgentSession::builder()
            .id(id)
            .name(name(agent))
            .tool(tool)
            .project(PathBuf::from(TEST_PROJECT))
            .launch_command(CommandLine::try_new(command).unwrap())
            .native_id(native_id.map(|native| NativeSessionId::try_new(native).unwrap()))
            .configured_key(Some(ConfiguredAgentKey::of(&name(agent)).unwrap()))
            .state(state)
            .build()
    }

    /// Records a launch owner and a captured Codex identity for session `id`.
    fn capture_codex(sessions: &SessionRecorder, id: &AgentSessionId) {
        let store = FakeAgentSessionStore {
            recorder: sessions.clone(),
        };
        store
            .set_owner_process_id(id, AgentProcessId::try_new(1).unwrap(), None, None)
            .unwrap();
        store
            .capture_native_id(
                id,
                AgentTool::Codex,
                NativeSessionId::try_new("codex-thread").unwrap(),
                AgentProcessId::try_new(1).unwrap(),
                None,
            )
            .unwrap();
    }

    /// The session id carried by `pane`'s process.
    fn session_of(harness: &Harness, pane: PaneId) -> AgentSessionId {
        harness
            .supervisor
            .workspace()
            .process(pane)
            .and_then(|process| process.agent_session_id().clone())
            .unwrap()
    }

    /// The state of `pane`'s process.
    fn state_of(harness: &Harness, pane: PaneId) -> ProcessState {
        *harness
            .supervisor
            .workspace()
            .process(pane)
            .unwrap()
            .state()
    }

    /// Delivers `outcome` as `pane`'s exit at its current generation.
    fn exit(harness: &mut Harness, pane: PaneId, outcome: ExitOutcome) -> OutputEffect {
        let generation = harness.supervisor.generations()[&pane];
        harness
            .supervisor
            .handle_output(pane, generation, ProcessOutput::Exited(outcome))
    }

    /// The state of the newest recorded session.
    fn last_state(sessions: &SessionRecorder) -> AgentSessionState {
        *sessions.sessions.borrow().last().unwrap().state()
    }

    /// A configured agent with no session is linked to a fresh durable session,
    /// so the shared spawn path resumes and captures it like a runtime session.
    #[test]
    fn a_configured_agent_is_linked_to_a_fresh_session() {
        let sessions = SessionRecorder::default();
        let mut harness = with_sessions(vec![configured_agent(0, "claude", "claude")], &sessions);

        harness.supervisor.link_configured_agent_sessions();

        let id = session_of(&harness, PaneId::new(0));
        let recorded = sessions.sessions.borrow();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].id(), &id);
        assert_eq!(recorded[0].name().as_ref(), "claude");
    }

    /// Re-linking a configured agent reuses its existing session by name and
    /// project rather than orphaning the conversation with a new one.
    #[test]
    fn a_configured_agent_reuses_its_existing_session() {
        let existing = configured_record("claude", AgentTool::Claude, "claude");
        let existing_id = existing.id().clone();
        let sessions = SessionRecorder {
            sessions: Rc::new(RefCell::new(vec![existing])),
        };
        let mut harness = with_sessions(vec![configured_agent(0, "claude", "claude")], &sessions);

        harness.supervisor.link_configured_agent_sessions();

        assert_eq!(session_of(&harness, PaneId::new(0)), existing_id);
        assert_eq!(
            sessions.sessions.borrow().len(),
            1,
            "no new session created"
        );
    }

    /// A previously launched reporter-based configured agent (Codex) with no
    /// captured native id must not be autostarted into a fresh conversation:
    /// automatic launches restore (no command, so it stays stopped), while an
    /// explicit user start reopens with a fresh session command.
    #[test]
    fn autostart_does_not_reopen_an_uncaptured_reporter_session() {
        let launched = configured_record("codex", AgentTool::Codex, "codex")
            .with_owner_process_id(AgentProcessId::try_new(4242).unwrap());
        let sessions = SessionRecorder {
            sessions: Rc::new(RefCell::new(vec![launched])),
        };
        let mut harness = with_sessions(vec![configured_agent(0, "codex", "codex")], &sessions);

        harness.supervisor.link_configured_agent_sessions();
        let pane = PaneId::new(0);

        assert!(
            matches!(
                harness.supervisor.command_of(pane, LaunchIntent::Automatic),
                Ok(None)
            ),
            "autostart must not silently reopen an uncaptured reporter session"
        );
        assert!(
            matches!(
                harness
                    .supervisor
                    .command_of(pane, LaunchIntent::UserInitiated),
                Ok(Some(_))
            ),
            "an explicit start reopens with a fresh session command"
        );
    }

    /// Distinctly named configured agents link to distinct sessions keyed by
    /// their names, so their conversations and ownership claims never collide.
    #[test]
    fn distinct_configured_agents_get_distinct_sessions() {
        let sessions = SessionRecorder::default();
        let mut harness = with_sessions(
            vec![
                configured_agent(0, "claude", "claude"),
                configured_agent(1, "opus", "claude"),
            ],
            &sessions,
        );

        harness.supervisor.link_configured_agent_sessions();

        let first = session_of(&harness, PaneId::new(0));
        let second = session_of(&harness, PaneId::new(1));
        assert_ne!(first, second, "distinct agents must not share a session");
        assert_eq!(sessions.sessions.borrow().len(), 2);
    }

    /// Changing a running configured agent's provider retains the live-owner
    /// gate, so the new provider cannot start and share the durable session
    /// while the old one is still running.
    #[test]
    fn changing_a_configured_provider_keeps_the_live_owner_gate() {
        let owner = AgentProcessId::try_new(std::process::id()).unwrap();
        let token = LocalProcessIdentity::start_token(owner).unwrap();
        let existing = configured_record_with(
            "claude",
            AgentTool::Claude,
            "claude",
            AgentSessionId::generate().unwrap(),
            Some("claude-thread"),
            AgentSessionState::Open,
        )
        .with_launch_owner(owner, Some(token), None);
        let existing_id = existing.id().clone();
        let sessions = SessionRecorder {
            sessions: Rc::new(RefCell::new(vec![existing])),
        };
        let mut harness = with_sessions(vec![configured_agent(0, "claude", "codex")], &sessions);

        harness.supervisor.link_configured_agent_sessions();

        let recorded = sessions.sessions.borrow();
        let session = recorded
            .iter()
            .find(|session| session.id() == &existing_id)
            .unwrap();
        assert_eq!(*session.tool(), AgentTool::Codex, "the provider is updated");
        assert!(
            session.native_id().is_none(),
            "the old provider's conversation cannot be resumed by the new one"
        );
        drop(recorded);
        assert!(
            harness
                .supervisor
                .agent_session_owned_by_live_process(PaneId::new(0)),
            "the retained live owner still gates the new provider's launch"
        );
    }

    /// A leftover configured record (its muster.yml row removed) is not
    /// restored as a runtime session, so a removed agent is not relaunched.
    #[test]
    fn a_removed_configured_agent_is_not_restored_as_a_session() {
        let leftover = configured_record_with(
            "claude",
            AgentTool::Claude,
            "claude",
            AgentSessionId::generate().unwrap(),
            Some("abc"),
            AgentSessionState::Open,
        );
        let sessions = SessionRecorder {
            sessions: Rc::new(RefCell::new(vec![leftover])),
        };
        let mut harness = with_sessions(Vec::new(), &sessions);

        harness.supervisor.restore_open_agent_sessions();

        assert!(
            harness.supervisor.workspace().processes().is_empty(),
            "a configured record must not be restored as a runtime session"
        );
    }

    /// Editing a configured agent's command refreshes its session's launch
    /// command while keeping the same durable session.
    #[test]
    fn a_configured_command_edit_refreshes_the_session() {
        let existing = configured_record("claude", AgentTool::Claude, "claude");
        let id = existing.id().clone();
        let sessions = SessionRecorder {
            sessions: Rc::new(RefCell::new(vec![existing])),
        };
        let mut harness = with_sessions(
            vec![configured_agent(0, "claude", "claude --model opus")],
            &sessions,
        );

        harness.supervisor.link_configured_agent_sessions();

        assert_eq!(
            session_of(&harness, PaneId::new(0)),
            id,
            "reuses the same durable session"
        );
        let recorded = sessions.sessions.borrow();
        let session = recorded.iter().find(|session| session.id() == &id).unwrap();
        assert_eq!(session.launch_command().as_ref(), "claude --model opus");
    }

    /// When a configured agent's session cannot be persisted it stays unlinked
    /// and `command_of` refuses to launch it into an untracked conversation.
    #[test]
    fn a_configured_agent_that_cannot_persist_is_not_launchable() {
        let mut harness = Harness::new(Box::new(FakeRunner), vec![configured_agent(
            0, "claude", "claude",
        )]);
        harness
            .supervisor
            .set_agent_session_store(Box::new(FailingAgentSessionStore));

        harness.supervisor.link_configured_agent_sessions();

        let pane = PaneId::new(0);
        assert!(
            harness
                .supervisor
                .workspace()
                .process(pane)
                .unwrap()
                .agent_session_id()
                .is_none(),
            "an unpersisted session must not bind the pane"
        );
        assert!(
            matches!(
                harness.supervisor.command_of(pane, LaunchIntent::Automatic),
                Ok(None)
            ),
            "an unlinked configured agent must not launch its raw command"
        );
    }

    /// A failed first spawn keeps a reported-ID session safely retryable.
    #[test]
    fn failed_initial_agent_spawn_remains_a_pending_fresh_launch() {
        let sessions = SessionRecorder::default();
        let mut harness = Harness::new(Box::new(FailingRunner), Vec::new());
        harness
            .supervisor
            .set_agent_session_store(Box::new(FakeAgentSessionStore {
                recorder: sessions.clone(),
            }));
        harness.supervisor.up();

        let row = harness
            .supervisor
            .create_agent_session(AgentTool::Codex, None, None, None)
            .unwrap();

        let session = sessions.sessions.borrow().last().cloned().unwrap();
        assert_eq!(state_of(&harness, row.pane()), ProcessState::Crashed);
        assert_eq!(*session.state(), AgentSessionState::Pending);
        assert_eq!(session.restore_command().unwrap().as_ref(), "codex");
    }

    /// New sessions are persisted before spawn and correlate provider hooks
    /// through an environment variable independent of their display name.
    #[test]
    fn launching_an_agent_session_persists_identity_and_exports_correlation() {
        let (mut harness, sessions, spawns) = agent_harness(Vec::new());

        let row = harness
            .supervisor
            .create_agent_session(AgentTool::Claude, None, None, None)
            .unwrap();

        assert!(matches!(row, SessionRow::Inserted(_)));
        let recorded = sessions.sessions.borrow();
        let session = recorded.first().unwrap();
        assert_eq!(*session.state(), AgentSessionState::Open);
        assert_eq!(session.project(), Path::new(TEST_PROJECT));
        assert!(session.native_id().is_none());
        let requests = spawns.requests.borrow();
        let request = requests.last().unwrap();
        assert_eq!(
            request
                .environment()
                .get(OsStr::new(MUSTER_AGENT_SESSION_ENV)),
            Some(&OsString::from(session.id().as_ref()))
        );
        assert_eq!(
            request.command().as_ref().unwrap().as_ref(),
            format!("claude --session-id {}", session.id()).as_str()
        );
    }

    /// A composed launch command needs no explicit resume template: the session
    /// is recorded and launched rather than refused.
    #[test]
    fn a_compound_agent_command_is_resumable_and_persisted() {
        let (mut harness, sessions, spawns) = agent_harness(Vec::new());

        harness
            .supervisor
            .create_agent_session(
                AgentTool::Codex,
                None,
                Some(CommandLine::try_new("codex | tee agent.log").unwrap()),
                None,
            )
            .unwrap();

        assert_eq!(
            sessions.sessions.borrow().len(),
            1,
            "the session is persisted"
        );
        assert_eq!(spawns.requests.borrow().len(), 1, "and launched");
    }

    /// An unsafe resume template is refused before anything is recorded or
    /// launched.
    #[test]
    fn an_invalid_resume_template_is_refused_before_launch() {
        let (mut harness, sessions, spawns) = agent_harness(Vec::new());

        let refused = harness.supervisor.create_agent_session(
            AgentTool::Codex,
            None,
            None,
            Some(CommandLine::try_new("agent --resume \"").unwrap()),
        );

        assert!(matches!(
            refused,
            Err(SupervisorError::ResumeTemplateInvalid)
        ));
        assert!(sessions.sessions.borrow().is_empty());
        assert!(spawns.requests.borrow().is_empty());
        assert!(harness.supervisor.workspace().processes().is_empty());
    }

    /// A custom agent starts its own command and keeps its resume template,
    /// and one without a command is refused before anything is recorded.
    #[test]
    fn a_custom_agent_runs_its_command_and_keeps_its_resume_template() {
        let (mut harness, sessions, spawns) = agent_harness(Vec::new());
        let template = CommandLine::try_new("mycodex resume {session_id}").unwrap();

        let commandless = harness
            .supervisor
            .new_agent_session(AgentTool::Custom, None, None, None);
        assert!(matches!(commandless, Err(SupervisorError::CommandRequired)));
        assert!(sessions.sessions.borrow().is_empty());

        let opened = harness
            .supervisor
            .new_agent_session(
                AgentTool::Custom,
                Some(name("wrapped")),
                Some(CommandLine::try_new("mycodex").unwrap()),
                Some(template.clone()),
            )
            .unwrap();

        assert_eq!(opened, name("wrapped"));
        let recorded = sessions.sessions.borrow();
        assert_eq!(recorded.len(), 1);
        assert_eq!(*recorded[0].tool(), AgentTool::Custom);
        assert_eq!(recorded[0].launch_command().as_ref(), "mycodex");
        assert_eq!(recorded[0].resume_command().as_ref(), Some(&template));
        assert_eq!(spawns.requests.borrow().len(), 1);
    }

    /// A new session from a preset starts under the given name, or a generated
    /// one, and reports the name it runs under.
    #[test]
    fn new_agent_session_starts_a_preset_under_its_name() {
        let (mut harness, _sessions, spawns) = agent_harness(Vec::new());

        let named = harness
            .supervisor
            .new_agent_session(AgentTool::Claude, Some(name("scout")), None, None)
            .unwrap();
        let generated = harness
            .supervisor
            .new_agent_session(AgentTool::Claude, None, None, None)
            .unwrap();

        assert_eq!(named, name("scout"));
        assert_ne!(generated, named);
        assert_eq!(spawns.requests.borrow().len(), 2);
        let status = harness.supervisor.snapshot();
        assert!(status.processes().iter().all(|process| {
            *process.kind() == ProcessKind::Agent && *process.state() == ProcessState::Running
        }));
    }

    /// A session cannot take a name another process uses, however its accent
    /// is encoded, since every command names one process by it.
    #[test]
    fn a_new_session_cannot_take_a_used_name() {
        let (mut harness, _sessions, spawns) = agent_harness(Vec::new());
        harness
            .supervisor
            .new_agent_session(AgentTool::Claude, Some(name("caf\u{e9}")), None, None)
            .unwrap();

        let taken = harness.supervisor.new_agent_session(
            AgentTool::Codex,
            Some(name("cafe\u{301}")),
            None,
            None,
        );

        assert!(matches!(taken, Err(SupervisorError::NameTaken(_))));
        assert_eq!(spawns.requests.borrow().len(), 1, "nothing else started");
    }

    /// A closed session whose name another session took meanwhile is not
    /// reopened beside it; the refusal names the name.
    #[test]
    fn reopening_a_session_whose_name_was_taken_is_refused() {
        let (mut harness, sessions, _spawns) = agent_harness(Vec::new());
        let pane = harness
            .supervisor
            .create_agent_session(AgentTool::Codex, Some(name("scout")), None, None)
            .unwrap()
            .pane();
        let id = session_of(&harness, pane);
        capture_codex(&sessions, &id);
        harness.supervisor.close_agent_session_pane(pane);
        exit(&mut harness, pane, ExitOutcome::Succeeded);
        harness
            .supervisor
            .new_agent_session(AgentTool::Claude, Some(name("scout")), None, None)
            .unwrap();

        let reopened = harness.supervisor.reopen_last_closed();

        match reopened {
            Err(error @ SupervisorError::NameTaken(_)) => {
                assert!(error.to_string().contains("'scout'"), "{error}");
            },
            other => panic!("the taken name is refused: {other:?}"),
        }
        assert_eq!(
            last_state(&sessions),
            AgentSessionState::Open,
            "the session that took the name is the open one"
        );
    }

    /// Closing a captured session moves it into history, and reopening uses the
    /// provider's native ID without duplicating its Muster identity.
    #[test]
    fn a_closed_agent_session_reopens_the_same_native_conversation() {
        let (mut harness, sessions, spawns) = agent_harness(Vec::new());
        let pane = harness
            .supervisor
            .create_agent_session(AgentTool::Codex, None, None, None)
            .unwrap()
            .pane();
        let id = session_of(&harness, pane);
        capture_codex(&sessions, &id);

        harness.supervisor.close_agent_session_pane(pane);
        exit(&mut harness, pane, ExitOutcome::Succeeded);
        assert_eq!(last_state(&sessions), AgentSessionState::Closed);

        let (reopened, outcome) = harness.supervisor.reopen_last_closed().unwrap();

        assert_eq!(reopened, name_of_session(&sessions, &id));
        assert!(matches!(outcome, ReopenOutcome::Opened(_)));
        assert_eq!(last_state(&sessions), AgentSessionState::Open);
        assert_eq!(
            harness
                .supervisor
                .workspace()
                .processes()
                .iter()
                .filter(|process| process.agent_session_id().as_ref() == Some(&id))
                .count(),
            1
        );
        assert_eq!(
            spawns
                .requests
                .borrow()
                .last()
                .unwrap()
                .command()
                .as_ref()
                .unwrap()
                .as_ref(),
            "codex resume codex-thread"
        );
    }

    /// The recorded name of session `id`.
    fn name_of_session(sessions: &SessionRecorder, id: &AgentSessionId) -> ProcessName {
        sessions
            .sessions
            .borrow()
            .iter()
            .find(|session| session.id() == id)
            .map(|session| session.name().clone())
            .unwrap()
    }

    /// Reopening during a delayed close waits for the retiring child before it
    /// creates the replacement row, which the exit reports.
    #[test]
    fn reopening_a_closing_session_respawns_after_its_exit() {
        let (mut harness, sessions, spawns) = agent_harness(Vec::new());
        let pane = harness
            .supervisor
            .create_agent_session(AgentTool::Codex, None, None, None)
            .unwrap()
            .pane();
        let id = session_of(&harness, pane);
        capture_codex(&sessions, &id);

        harness.supervisor.close_agent_session_pane(pane);
        let session = harness.supervisor.last_closed_session().unwrap();
        let outcome = harness
            .supervisor
            .reopen_agent_session(session.id())
            .unwrap();

        assert_eq!(outcome, ReopenOutcome::Queued);
        assert_eq!(spawns.requests.borrow().len(), 1);
        assert_eq!(last_state(&sessions), AgentSessionState::Open);
        assert_eq!(
            harness.supervisor.pending_session_reopens.get(&pane),
            Some(&id)
        );

        let effect = exit(&mut harness, pane, ExitOutcome::Succeeded);

        let OutputEffect::Exited {
            retired: true,
            reopened: Some(SessionRow::Inserted(reopened)),
        } = effect
        else {
            panic!("the exit retires the old row and reports the reopened one");
        };
        assert_eq!(spawns.requests.borrow().len(), 2);
        assert!(harness.supervisor.pending_session_reopens.is_empty());
        assert_eq!(session_of(&harness, reopened), id);
    }

    /// Closing a session again cancels a reopen queued while its child exits.
    #[test]
    fn closing_again_cancels_a_queued_session_reopen() {
        let (mut harness, sessions, spawns) = agent_harness(Vec::new());
        let pane = harness
            .supervisor
            .create_agent_session(AgentTool::Codex, None, None, None)
            .unwrap()
            .pane();
        let id = session_of(&harness, pane);
        capture_codex(&sessions, &id);

        harness.supervisor.close_agent_session_pane(pane);
        let (_, outcome) = harness.supervisor.reopen_last_closed().unwrap();
        assert_eq!(outcome, ReopenOutcome::Queued);
        assert_eq!(
            harness.supervisor.pending_session_reopens.get(&pane),
            Some(&id)
        );

        harness.supervisor.close_agent_session_pane(pane);
        let effect = exit(&mut harness, pane, ExitOutcome::Succeeded);

        assert!(matches!(effect, OutputEffect::Exited {
            retired: true,
            reopened: None
        }));
        assert_eq!(spawns.requests.borrow().len(), 1);
        assert!(harness.supervisor.pending_session_reopens.is_empty());
        assert_eq!(last_state(&sessions), AgentSessionState::Closed);
    }

    /// A store read failure after exit settles the pane instead of leaving it
    /// restarting, and says why.
    #[test]
    fn restart_after_exit_surfaces_a_session_store_failure() {
        let (mut harness, _sessions, _spawns) = agent_harness(Vec::new());
        let pane = harness
            .supervisor
            .create_agent_session(AgentTool::Claude, None, None, None)
            .unwrap()
            .pane();
        harness.supervisor.panes.get_mut(&pane).unwrap().exit_intent = ExitIntent::RestartInFlight;
        harness
            .supervisor
            .set_agent_session_store(Box::new(UnreadableAgentSessionStore));

        exit(&mut harness, pane, ExitOutcome::FAILED_UNKNOWN);

        assert_eq!(state_of(&harness, pane), ProcessState::Crashed);
        assert!(
            harness
                .notice_messages()
                .last()
                .is_some_and(|notice| notice.starts_with(AGENT_SESSION_STORE_ERROR))
        );
    }

    /// Reopen skips newer history that lacks a provider identity only when it
    /// cannot reopen fresh: an uncaptured newer session reopens fresh.
    #[test]
    fn reopen_uses_the_latest_closed_session_reopening_fresh_when_uncaptured() {
        let older = persisted_agent_session(
            AgentTool::Codex,
            "captured-thread",
            AgentSessionState::Closed,
        );
        let newer = AgentSession::builder()
            .id(AgentSessionId::generate().unwrap())
            .name(name("Grace"))
            .tool(AgentTool::Gemini)
            .project(PathBuf::from(TEST_PROJECT))
            .launch_command(CommandLine::try_new("gemini").unwrap())
            .state(AgentSessionState::Closed)
            .build()
            .with_owner_process_id(AgentProcessId::try_new(4242).unwrap());
        let newer_id = newer.id().clone();
        let expected = AgentTool::Gemini
            .new_session_command(newer.launch_command(), newer.id())
            .unwrap();
        let (mut harness, _sessions, spawns) = agent_harness(vec![older, newer]);

        assert_eq!(
            harness.supervisor.reopen_last_closed().unwrap().0,
            name("Grace")
        );

        assert_eq!(
            harness.supervisor.workspace().processes()[0]
                .agent_session_id()
                .as_ref(),
            Some(&newer_id)
        );
        assert_eq!(
            spawns.requests.borrow()[0].command().as_ref().unwrap(),
            &expected
        );
    }

    /// With no closed session to reopen, the request says so and starts nothing.
    #[test]
    fn reopening_with_no_closed_session_is_refused() {
        let (mut harness, _sessions, spawns) = agent_harness(Vec::new());

        assert!(matches!(
            harness.supervisor.reopen_last_closed(),
            Err(SupervisorError::NoRecentSession)
        ));
        assert!(spawns.requests.borrow().is_empty());
    }

    /// Startup restores open sessions with their resume command but leaves
    /// closed history dormant.
    #[test]
    fn startup_restores_only_open_agent_sessions() {
        let open =
            persisted_agent_session(AgentTool::Gemini, "gemini-open", AgentSessionState::Open);
        let open_id = open.id().clone();
        let closed = persisted_agent_session(
            AgentTool::Copilot,
            "copilot-closed",
            AgentSessionState::Closed,
        );

        let (harness, _sessions, spawns) = agent_harness(vec![open, closed]);

        let processes = harness.supervisor.workspace().processes();
        assert_eq!(processes.len(), 1);
        assert_eq!(processes[0].agent_session_id().as_ref(), Some(&open_id));
        assert_eq!(
            spawns.requests.borrow()[0]
                .command()
                .as_ref()
                .unwrap()
                .as_ref(),
            "gemini --resume gemini-open"
        );
        assert!(*processes[0].autostart());
    }

    /// A second Muster instance leaves a session alone while its owner lives.
    #[test]
    fn startup_skips_an_open_session_with_a_live_owner() {
        let owner = AgentProcessId::try_new(std::process::id()).unwrap();
        let token = LocalProcessIdentity::start_token(owner).unwrap();
        let session =
            persisted_agent_session(AgentTool::Codex, "owned-thread", AgentSessionState::Open)
                .with_launch_owner(owner, Some(token), None);

        let (harness, _sessions, spawns) = agent_harness(vec![session]);

        assert!(harness.supervisor.workspace().processes().is_empty());
        assert!(spawns.requests.borrow().is_empty());
    }

    /// A numeric PID without its creation token cannot suppress restoration,
    /// because it may already belong to an unrelated process.
    #[test]
    fn startup_restores_a_session_with_an_unverifiable_owner() {
        let owner = AgentProcessId::try_new(std::process::id()).unwrap();
        let session = persisted_agent_session(
            AgentTool::Codex,
            "unverifiable-owner-thread",
            AgentSessionState::Open,
        )
        .with_owner_process_id(owner);

        let (harness, _sessions, spawns) = agent_harness(vec![session]);

        assert_eq!(harness.supervisor.workspace().processes().len(), 1);
        assert_eq!(spawns.requests.borrow().len(), 1);
    }

    /// An unconfirmed caller-assigned identity retries its original launch
    /// command instead of resuming a conversation the provider never created.
    #[test]
    fn startup_retries_an_unconfirmed_assigned_agent_session() {
        let session = AgentSession::builder()
            .id(AgentSessionId::try_new("assigned-session").unwrap())
            .name(name("Ada"))
            .tool(AgentTool::Claude)
            .project(PathBuf::from(TEST_PROJECT))
            .launch_command(CommandLine::try_new("claude").unwrap())
            .state(AgentSessionState::Open)
            .build();
        let id = session.id().clone();

        let (harness, _sessions, spawns) = agent_harness(vec![session]);

        assert_eq!(
            harness.supervisor.workspace().processes()[0]
                .agent_session_id()
                .as_ref(),
            Some(&id)
        );
        assert_eq!(
            spawns.requests.borrow()[0]
                .command()
                .as_ref()
                .unwrap()
                .as_ref(),
            "claude --session-id assigned-session"
        );
    }

    /// Spawning a session pane synchronously claims the launcher as owner, so
    /// no reader can see an owner-less open session while it is live.
    #[test]
    fn spawning_a_session_pane_claims_the_owner_synchronously() {
        let sessions = SessionRecorder::default();
        let mut harness = Harness::new(Box::new(PidRunner), Vec::new());
        harness
            .supervisor
            .set_agent_session_store(Box::new(FakeAgentSessionStore {
                recorder: sessions.clone(),
            }));
        harness.supervisor.up();

        harness
            .supervisor
            .create_agent_session(AgentTool::Codex, None, None, None)
            .unwrap();

        assert_eq!(
            sessions.sessions.borrow()[0]
                .owner_process_id()
                .map(|pid| pid.into_inner()),
            Some(FAKE_LAUNCHER_PID),
            "the owner is claimed in the spawn call stack"
        );
    }

    /// A never-launched restored session starts a fresh conversation rather
    /// than being stuck behind the not-resumable notice.
    #[test]
    fn a_never_launched_session_restores_ready_to_start_fresh() {
        let session = AgentSession::builder()
            .id(AgentSessionId::generate().unwrap())
            .name(name("Rahul"))
            .tool(AgentTool::Codex)
            .project(PathBuf::from(TEST_PROJECT))
            .launch_command(CommandLine::try_new("codex").unwrap())
            .state(AgentSessionState::Open)
            .build();
        let id = session.id().clone();

        let (harness, _sessions, spawns) = agent_harness(vec![session]);

        assert_eq!(
            harness.supervisor.workspace().processes()[0]
                .agent_session_id()
                .as_ref(),
            Some(&id)
        );
        assert!(
            harness.notice_messages().is_empty(),
            "no not-resumable notice: {:?}",
            harness.notice_messages()
        );
        assert!(
            !spawns.requests.borrow().is_empty(),
            "the session starts fresh"
        );
    }

    /// An open session that ran but whose provider ID was never captured
    /// returns as a stopped row, with a notice, that can be closed.
    #[test]
    fn startup_restores_an_uncaptured_session_as_a_closable_stopped_row() {
        let session = AgentSession::builder()
            .id(AgentSessionId::generate().unwrap())
            .name(name("Ada"))
            .tool(AgentTool::Codex)
            .project(PathBuf::from(TEST_PROJECT))
            .launch_command(CommandLine::try_new("codex").unwrap())
            .state(AgentSessionState::Open)
            .build()
            .with_owner_process_id(AgentProcessId::try_new(4242).unwrap());
        let id = session.id().clone();

        let (mut harness, sessions, spawns) = agent_harness(vec![session]);

        let pane = *harness.supervisor.workspace().processes()[0].id();
        assert_eq!(session_of(&harness, pane), id);
        assert_eq!(state_of(&harness, pane), ProcessState::Pending);
        assert!(!harness.supervisor.panes().contains_key(&pane));
        assert!(spawns.requests.borrow().is_empty());
        assert_eq!(harness.notice_messages(), vec![
            AGENT_SESSION_NOT_RESUMABLE.to_string()
        ]);

        harness.supervisor.close_agent_session(&name("Ada"));

        assert!(harness.supervisor.workspace().process(pane).is_none());
        assert_eq!(last_state(&sessions), AgentSessionState::Closed);
    }

    /// Only a runtime agent session can be closed by name; anything else is
    /// refused as a notice and left alone.
    #[test]
    fn closing_by_name_refuses_anything_but_an_agent_session() {
        let terminal = Process::builder()
            .id(PaneId::new(0))
            .name(name("shell"))
            .kind(ProcessKind::Terminal)
            .build();
        let mut harness = Harness::new(Box::new(FakeRunner), vec![terminal]);

        harness.supervisor.close_agent_session(&name("shell"));
        harness.supervisor.close_agent_session(&name("ghost"));

        assert!(
            harness
                .supervisor
                .workspace()
                .process(PaneId::new(0))
                .is_some()
        );
        assert_eq!(harness.notice_messages(), vec![
            NOT_AN_AGENT_SESSION.to_string(),
            UNKNOWN_PROCESS_NOTICE.to_string(),
        ]);
    }

    /// A failed hard-stop leaves durable history open and the live pane
    /// tracked, while a later successful retry closes and schedules that pane to
    /// retire.
    #[test]
    fn failed_agent_session_stop_does_not_persist_a_close() {
        let (mut harness, sessions, _spawns) = agent_harness(Vec::new());
        let pane = harness
            .supervisor
            .create_agent_session(AgentTool::Claude, None, None, None)
            .unwrap()
            .pane();
        let signals = Arc::new(RetryStopSignals::default());
        harness.supervisor.panes.get_mut(&pane).unwrap().handle = Some(Box::new(RetryStopHandle {
            signals: signals.clone(),
            terminate_outcome: TerminateOutcome::Fails,
        }));

        harness.supervisor.close_agent_session_pane(pane);

        let membership = |harness: &Harness| harness.supervisor.panes()[&pane].config_membership;
        assert_eq!(signals.kills.load(Ordering::SeqCst), 1);
        assert_eq!(last_state(&sessions), AgentSessionState::Open);
        assert_eq!(membership(&harness), ConfigMembership::Tracked);
        assert!(harness.supervisor.is_live(pane));
        assert_eq!(state_of(&harness, pane), ProcessState::Running);
        assert_eq!(
            harness.notice_messages().last().map(String::as_str),
            Some(STOP_DELIVERY_FAILED_NOTICE)
        );

        harness.supervisor.close_agent_session_pane(pane);

        assert_eq!(signals.kills.load(Ordering::SeqCst), 2);
        assert_eq!(last_state(&sessions), AgentSessionState::Closed);
        assert_eq!(membership(&harness), ConfigMembership::RetireOnExit);
        assert_eq!(state_of(&harness, pane), ProcessState::Stopping);
    }

    /// A delivered stop with failed durable persistence remains a tracked row
    /// and an open history record, so its eventual exit cannot lose the session.
    #[test]
    fn failed_session_close_persistence_keeps_the_session_recoverable() {
        let (mut harness, sessions, _spawns) = agent_harness(Vec::new());
        let pane = harness
            .supervisor
            .create_agent_session(AgentTool::Claude, None, None, None)
            .unwrap()
            .pane();
        harness
            .supervisor
            .set_agent_session_store(Box::new(FailingAgentSessionStore));

        harness.supervisor.close_agent_session_pane(pane);

        assert_eq!(last_state(&sessions), AgentSessionState::Open);
        assert_eq!(
            harness.supervisor.panes()[&pane].config_membership,
            ConfigMembership::Tracked
        );
        assert_eq!(state_of(&harness, pane), ProcessState::Stopping);
        assert!(
            harness
                .notice_messages()
                .last()
                .is_some_and(|notice| notice.contains(AGENT_SESSION_STORE_ERROR))
        );
    }

    /// A configured session with no matching config agent, as one left behind
    /// when its agent was removed from muster.yml, is returned to history on
    /// startup.
    #[test]
    fn a_stranded_configured_session_is_recovered_on_startup() {
        let orphan = configured_record_with(
            "backend",
            AgentTool::Claude,
            "claude",
            AgentSessionId::generate().unwrap(),
            Some("conv-42"),
            AgentSessionState::Closed,
        );

        let (_harness, sessions, _spawns) = agent_harness(vec![orphan.clone()]);

        let recorded = sessions.sessions.borrow();
        let record = recorded
            .iter()
            .find(|session| session.id() == orphan.id())
            .unwrap();
        assert!(record.configured_key().is_none());
        assert_eq!(record.native_id(), orphan.native_id());
    }

    /// The registry the re-added-agent recovery tests run against: the latest
    /// config lists `Ada`, through the plain read when `locked_only` is unset,
    /// otherwise only through the locked read.
    struct ReaddedRegistry {
        locked_only: bool,
    }

    impl ReaddedRegistry {
        /// The config with `Ada` re-added.
        fn readded() -> WorkspaceConfig {
            empty_workspace_config().with_agents(vec![
                ProcessSpec::builder()
                    .name(name("Ada"))
                    .command(Some(CommandLine::try_new("claude").unwrap()))
                    .autostart(true)
                    .build(),
            ])
        }
    }

    impl ProjectRegistry for ReaddedRegistry {
        fn projects(&self) -> Result<Vec<Project>, ConfigError> {
            Ok(Vec::new())
        }

        fn workspace(&self, _config_path: &Path) -> Result<WorkspaceConfig, ConfigError> {
            Ok(if self.locked_only {
                empty_workspace_config()
            } else {
                Self::readded()
            })
        }

        fn workspace_exists(&self, _config_path: &Path) -> bool {
            true
        }

        fn save(&self, _projects: &[Project]) -> Result<(), ConfigError> {
            Ok(())
        }

        fn save_workspace(
            &self,
            _config_path: &Path,
            _config: &WorkspaceConfig,
        ) -> Result<(), ConfigError> {
            Ok(())
        }

        fn with_workspace_locked(
            &self,
            _config_path: &Path,
            read: &mut dyn FnMut(&WorkspaceConfig) -> Result<(), ConfigError>,
        ) -> Result<(), ConfigError> {
            read(&Self::readded())
        }
    }

    /// Recovers orphaned configured sessions over an empty workspace whose config re-added
    /// `Ada`, returning her session afterwards.
    fn recover_readded(registry: ReaddedRegistry) -> AgentSession {
        let sessions = SessionRecorder {
            sessions: Rc::new(RefCell::new(vec![configured_record_with(
                "Ada",
                AgentTool::Claude,
                "claude",
                AgentSessionId::generate().unwrap(),
                None,
                AgentSessionState::Open,
            )])),
        };
        let mut harness =
            Harness::with_registry(Box::new(FakeRunner), Vec::new(), Box::new(registry));
        harness
            .supervisor
            .set_agent_session_store(Box::new(FakeAgentSessionStore {
                recorder: sessions.clone(),
            }));

        harness.supervisor.recover_orphaned_configured_sessions();

        sessions
            .sessions
            .borrow()
            .iter()
            .find(|session| session.name() == &name("Ada"))
            .cloned()
            .unwrap()
    }

    /// A configured agent removed locally but re-added to muster.yml by another
    /// instance keeps its session through orphan recovery.
    #[test]
    fn a_concurrently_readded_agents_session_survives_orphan_recovery() {
        let ada = recover_readded(ReaddedRegistry { locked_only: false });

        assert!(ada.configured_key().is_some());
        assert_ne!(*ada.state(), AgentSessionState::Closed);
    }

    /// Orphan recovery checks the config under the workspace lock, so an agent
    /// visible only through the locked read is preserved.
    #[test]
    fn orphan_recovery_reads_the_config_under_the_workspace_lock() {
        let ada = recover_readded(ReaddedRegistry { locked_only: true });

        assert!(ada.configured_key().is_some());
        assert_ne!(*ada.state(), AgentSessionState::Closed);
    }

    /// A supervisor over no processes whose launches run on a worker, with
    /// durable history held by `sessions`.
    fn launching_on_a_worker(runner: WorkerRunner, sessions: &SessionRecorder) -> Harness {
        let mut harness = Harness::new(Box::new(runner), Vec::new());
        harness
            .supervisor
            .set_agent_session_store(Box::new(FakeAgentSessionStore {
                recorder: sessions.clone(),
            }));
        harness
    }

    /// The recorded state of session `id`.
    fn state_of_session(sessions: &SessionRecorder, id: &AgentSessionId) -> AgentSessionState {
        *sessions
            .sessions
            .borrow()
            .iter()
            .find(|session| session.id() == id)
            .unwrap()
            .state()
    }

    /// A session closed while its launch is starting stays closed: the row
    /// retires at once, the launched process is killed when it arrives, and
    /// the next start of the project does not bring the session back.
    #[test]
    fn closing_a_session_while_its_launch_starts_keeps_it_closed() {
        let sessions = SessionRecorder::default();
        let runner = WorkerRunner::default();
        let kills = Arc::clone(&runner.0);
        let mut harness = launching_on_a_worker(runner, &sessions);
        let pane = harness
            .supervisor
            .create_agent_session(AgentTool::Claude, None, None, None)
            .unwrap()
            .pane();
        let id = session_of(&harness, pane);
        assert_eq!(state_of(&harness, pane), ProcessState::Starting);

        harness.supervisor.close_agent_session_pane(pane);
        let (pane, generation, result) = spawned(&harness);
        harness.supervisor.handle_spawned(pane, generation, result);

        assert_eq!(state_of_session(&sessions, &id), AgentSessionState::Closed);
        assert_eq!(
            kills.load(Ordering::SeqCst),
            1,
            "the arriving launch is killed"
        );
        assert!(!harness.supervisor.is_live(pane));
        assert!(harness.supervisor.workspace().processes().is_empty());
        let mut next = launching_on_a_worker(WorkerRunner::default(), &sessions);
        next.supervisor.up();
        assert!(
            next.supervisor.workspace().processes().is_empty(),
            "a closed session is never restored"
        );
        assert_eq!(state_of_session(&sessions, &id), AgentSessionState::Closed);
    }
}
