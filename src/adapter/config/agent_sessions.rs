#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt};
use std::{
    fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
};

use atomic_write_file::AtomicWriteFile;
#[cfg(unix)]
use atomic_write_file::unix::OpenOptionsExt as _;
use serde::{Deserialize, Serialize};

use super::yaml::state_dir_path;
use crate::{
    adapter::process_identity::LocalProcessIdentity,
    constants::MUSTER_AGENT_SESSION_STATE_FILE_ENV,
    domain::{
        agent_session::{
            AgentProcessId, AgentProcessStartToken, AgentSession, AgentSessionId,
            AgentSessionState, ConfiguredAgentKey, LaunchToken, NativeSessionId,
        },
        config::ConfigError,
        port::AgentSessionStore,
        process::AgentTool,
    },
};

/// Agent-session state filename under muster's platform state directory.
const AGENT_SESSIONS_FILE: &str = "agent-sessions.yml";
/// Current on-disk schema version.
const SESSION_FILE_VERSION: u8 = 1;
/// Maximum symlink chain followed for the durable session-state file.
const MAX_SESSION_STATE_SYMLINKS: usize = 40;
/// New session-state files are readable and writable only by their owner.
#[cfg(unix)]
const PRIVATE_SESSION_FILE_MODE: u32 = 0o600;
/// Permission bits retained when inspecting an existing Unix file.
#[cfg(unix)]
const FILE_PERMISSION_MASK: u32 = 0o777;
/// Existing modes are preserved only when they grant no group or other access.
#[cfg(unix)]
const OWNER_PERMISSION_MASK: u32 = 0o700;

/// Versioned on-disk agent-session history.
#[derive(Serialize, Deserialize)]
struct SessionFile {
    version: u8,
    sessions: Vec<AgentSession>,
}

impl SessionFile {
    /// Creates an empty file at the current schema version.
    fn empty() -> Self {
        Self {
            version: SESSION_FILE_VERSION,
            sessions: Vec::new(),
        }
    }
}

/// YAML-backed session history shared by the TUI and provider hooks.
#[derive(Default)]
pub struct YamlAgentSessionStore;

impl YamlAgentSessionStore {
    /// Resolves the platform state-file path.
    fn path() -> Option<PathBuf> {
        std::env::var_os(MUSTER_AGENT_SESSION_STATE_FILE_ENV)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| state_dir_path(AGENT_SESSIONS_FILE))
    }

    /// Loads `path` without acquiring its sibling lock.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] for I/O, YAML, or schema-version failures.
    fn load_from(path: &Path) -> Result<SessionFile, ConfigError> {
        if !path.exists() {
            return Ok(SessionFile::empty());
        }
        let raw = fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let file: SessionFile = serde_yaml_ng::from_str(&raw)?;
        if file.version != SESSION_FILE_VERSION {
            return Err(ConfigError::UnsupportedAgentSessionVersion(file.version));
        }
        Ok(file)
    }

    /// Mutates the state file under one cross-process advisory lock.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if locking, loading, mutation, or writing fails.
    fn update(
        path: &Path,
        mutate: impl FnOnce(&mut Vec<AgentSession>) -> Result<(), ConfigError>,
    ) -> Result<(), ConfigError> {
        let path = Self::write_destination(path)?;
        let _guard = Self::lock(&path)?;
        let mut file = Self::load_from(&path)?;
        mutate(&mut file.sessions)?;
        Self::write(&path, &file)
    }

    /// Resolves a state-file symlink without requiring its final target to
    /// exist, preserving aliases during atomic replacement.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when a link cannot be resolved safely.
    fn write_destination(path: &Path) -> Result<PathBuf, ConfigError> {
        let mut destination = path.to_path_buf();
        for depth in 0..=MAX_SESSION_STATE_SYMLINKS {
            match fs::symlink_metadata(&destination) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    if depth == MAX_SESSION_STATE_SYMLINKS {
                        return Self::symlink_depth_error(destination);
                    }
                    let target =
                        fs::read_link(&destination).map_err(|source| ConfigError::Read {
                            path: destination.clone(),
                            source,
                        })?;
                    destination = if target.is_absolute() {
                        target
                    } else {
                        destination
                            .parent()
                            .map_or(target.clone(), |parent| parent.join(target))
                    };
                },
                Ok(_) => return Ok(destination),
                Err(error) if error.kind() == ErrorKind::NotFound => return Ok(destination),
                Err(source) => {
                    return Err(ConfigError::Read {
                        path: destination,
                        source,
                    });
                },
            }
        }
        Self::symlink_depth_error(destination)
    }

    /// Creates a descriptive read error for a cyclic state-file symlink chain.
    fn symlink_depth_error(path: PathBuf) -> Result<PathBuf, ConfigError> {
        Err(ConfigError::Read {
            path,
            source: std::io::Error::other("agent session-state symlink depth exceeded"),
        })
    }

    /// Serializes and atomically replaces the session file without ever
    /// exposing its contents to group or other users on Unix.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if serialization, directory creation, file
    /// metadata access, writing, or replacement fails.
    fn write(path: &Path, value: &SessionFile) -> Result<(), ConfigError> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let raw = serde_yaml_ng::to_string(value)?;
        let mut options = AtomicWriteFile::options();
        #[cfg(unix)]
        {
            options.preserve_mode(false);
            options.mode(PRIVATE_SESSION_FILE_MODE);
        }
        let mut file = options.open(path).map_err(|source| ConfigError::Write {
            path: path.to_path_buf(),
            source,
        })?;
        #[cfg(unix)]
        file.set_permissions(fs::Permissions::from_mode(Self::secure_file_mode(path)?))
            .map_err(|source| ConfigError::Write {
                path: path.to_path_buf(),
                source,
            })?;
        file.write_all(raw.as_bytes())
            .map_err(|source| ConfigError::Write {
                path: path.to_path_buf(),
                source,
            })?;
        file.commit().map_err(|source| ConfigError::Write {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Returns an existing owner-only mode unchanged and narrows any broader
    /// Unix mode to the private default.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when metadata fails for a reason other than a
    /// missing destination.
    #[cfg(unix)]
    fn secure_file_mode(path: &Path) -> Result<u32, ConfigError> {
        match fs::metadata(path) {
            Ok(metadata) => {
                let mode = metadata.permissions().mode() & FILE_PERMISSION_MASK;
                Ok(if mode & !OWNER_PERMISSION_MASK == 0 {
                    mode
                } else {
                    PRIVATE_SESSION_FILE_MODE
                })
            },
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(PRIVATE_SESSION_FILE_MODE),
            Err(source) => Err(ConfigError::Read {
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    /// Acquires the stable cross-platform sibling lock shared by TUI and hook
    /// writers.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] if the lock file cannot be created or locked.
    fn lock(path: &Path) -> Result<fs::File, ConfigError> {
        let lock_path = Self::lock_path(path);
        if let Some(parent) = lock_path.parent() {
            fs::create_dir_all(parent).map_err(|source| ConfigError::Write {
                path: parent.to_path_buf(),
                source,
            })?;
        }
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&lock_path)
            .map_err(|source| ConfigError::Write {
                path: lock_path.clone(),
                source,
            })?;
        file.lock().map_err(|source| ConfigError::Write {
            path: lock_path,
            source,
        })?;
        Ok(file)
    }

    /// Builds the stable sibling lock path for `path`.
    fn lock_path(path: &Path) -> PathBuf {
        let mut name = path.file_name().unwrap_or_default().to_os_string();
        name.push(".lock");
        path.with_file_name(name)
    }

    /// Moves an updated record to the newest history position.
    fn replace(sessions: &mut Vec<AgentSession>, session: AgentSession) {
        sessions.retain(|candidate| candidate.id() != session.id());
        sessions.push(session);
    }

    /// Whether `session` is the durable record for the same configured agent as
    /// `other`: same owning project and same (present) configured key.
    fn same_configured_agent(session: &AgentSession, other: &AgentSession) -> bool {
        session.project() == other.project()
            && session.configured_key().is_some()
            && session.configured_key() == other.configured_key()
    }

    /// Claims a session for a newly launched provider unless a verified live
    /// owner already holds it.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the session is absent or owned by another
    /// live provider.
    fn claim_owner(
        session: &mut AgentSession,
        id: &AgentSessionId,
        process_id: AgentProcessId,
        process_start_token: Option<AgentProcessStartToken>,
        launch_token: Option<LaunchToken>,
    ) -> Result<(), ConfigError> {
        if let (Some(owner), Some(token)) = (
            session.owner_process_id(),
            session.owner_process_start_token(),
        ) && LocalProcessIdentity::start_token(*owner) == Some(*token)
            && *owner != process_id
        {
            return Err(ConfigError::AgentSessionAlreadyOwned {
                id: id.clone(),
                owner: *owner,
            });
        }
        // The launch token is recorded only past this ownership check, so a rejected
        // claim leaves the owning launch's token untouched.
        *session = session
            .clone()
            .with_launch_owner(process_id, process_start_token, launch_token);
        Ok(())
    }

    /// Records the conversation identity a session's provider reported. The report
    /// is trusted by session id: muster injects the id into the launched process's
    /// environment, so only that process tree can produce it, whatever launcher
    /// layers sit between muster and the agent (a node shim below codex, say).
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the session is absent or the lifecycle event
    /// came from a different provider.
    fn assign_native_id(
        sessions: &mut [AgentSession],
        id: &AgentSessionId,
        provider: AgentTool,
        native_id: NativeSessionId,
        reporter_process_id: AgentProcessId,
        reported_launch_token: Option<LaunchToken>,
    ) -> Result<(), ConfigError> {
        let session = sessions
            .iter_mut()
            .find(|session| session.id() == id)
            .ok_or_else(|| ConfigError::AgentSessionNotFound(id.clone()))?;
        if *session.tool() != provider {
            return Err(ConfigError::AgentSessionProviderMismatch {
                id: id.clone(),
                expected: *session.tool(),
                reported: provider,
            });
        }
        // Once a launch has recorded its token, only a report carrying that exact
        // token binds. A stale capture from a previous launch (a different token) or
        // from a pre-token launch (no token) is rejected, so it cannot claim the
        // reporter slot and lock out this launch's real, differently-PID'd report. A
        // session with no token yet (a legacy launch) still accepts a tokenless
        // report.
        if let Some(current) = session.launch_token()
            && reported_launch_token.as_ref() != Some(current)
        {
            return Ok(());
        }
        // Bind the identity to the process that reports it this launch. A different
        // process keeps the lock unless the one that bound the identity is confirmed
        // no longer the live original: it has exited (a wrapper that keeps the launch
        // alive by restarting the provider yields a new PID under the same token), or
        // its PID was reused by a different process (a changed start token). Uncertain
        // liveness or an unavailable start token keeps the lock rather than failing
        // open, and each launch resets the reporter.
        if let Some(current) = session.native_reporter_process_id()
            && *current != reporter_process_id
        {
            let replaced = if LocalProcessIdentity::is_alive(*current) {
                matches!(
                    (
                        session.native_reporter_start_token(),
                        LocalProcessIdentity::start_token(*current),
                    ),
                    (Some(recorded), Some(now)) if *recorded != now
                )
            } else {
                true
            };
            if !replaced {
                return Ok(());
            }
        }
        let reporter_start_token = LocalProcessIdentity::start_token(reporter_process_id);
        *session = session.clone().with_reported_native_id(
            native_id,
            reporter_process_id,
            reporter_start_token,
        );
        Ok(())
    }
}

impl AgentSessionStore for YamlAgentSessionStore {
    fn sessions(&self) -> Result<Vec<AgentSession>, ConfigError> {
        let path = Self::path().ok_or(ConfigError::NoConfigDir)?;
        let path = Self::write_destination(&path)?;
        Ok(Self::load_from(&path)?.sessions)
    }

    fn state_file_path(&self) -> Result<Option<PathBuf>, ConfigError> {
        Ok(Self::path())
    }

    fn upsert(&self, session: &AgentSession) -> Result<(), ConfigError> {
        let path = Self::path().ok_or(ConfigError::NoConfigDir)?;
        Self::update(&path, |sessions| {
            Self::replace(sessions, session.clone());
            Ok(())
        })
    }

    fn link_configured(&self, candidate: &AgentSession) -> Result<AgentSessionId, ConfigError> {
        let path = Self::path().ok_or(ConfigError::NoConfigDir)?;
        // Default to the create case; overwritten to the existing id on reuse.
        let mut resolved = candidate.id().clone();
        Self::update(&path, |sessions| {
            let existing = sessions
                .iter_mut()
                .find(|session| Self::same_configured_agent(session, candidate));
            match existing {
                Some(session) => {
                    *session = session.clone().with_configured_command(
                        *candidate.tool(),
                        candidate.launch_command().clone(),
                    );
                    resolved = session.id().clone();
                },
                None => sessions.push(candidate.clone()),
            }
            Ok(())
        })?;
        Ok(resolved)
    }

    fn pin_configured(&self, session: &AgentSession) -> Result<Vec<AgentSession>, ConfigError> {
        let path = Self::path().ok_or(ConfigError::NoConfigDir)?;
        let mut displaced = Vec::new();
        Self::update(&path, |sessions| {
            // Under the lock, the source must still be unconfigured, or already this
            // exact target (an idempotent retry). A source a concurrent instance
            // already pinned to a different agent must not be transferred again:
            // doing so would clobber that agent's session and orphan its seed.
            if let Some(current) = sessions
                .iter()
                .find(|candidate| candidate.id() == session.id())
                && current.configured_key().is_some()
                && !Self::same_configured_agent(current, session)
            {
                return Err(ConfigError::AgentSessionAlreadyPinned(session.id().clone()));
            }
            // Overlay the live runtime state of the record being transferred, read
            // fresh under the lock, so the transfer cannot erase a still-live owner
            // (letting autostart run a competing provider) or lose an identity a
            // concurrent reopen captured after the caller's snapshot was taken.
            let record = sessions
                .iter()
                .find(|candidate| candidate.id() == session.id())
                .map_or_else(
                    || session.clone(),
                    |current| session.clone().with_runtime_state_of(current),
                );
            // Displaced records (the source and any prior record for this target)
            // are returned so a failed follow-up can restore the full pre-pin state.
            let (removed, kept): (Vec<_>, Vec<_>) = sessions.drain(..).partition(|candidate| {
                candidate.id() == session.id() || Self::same_configured_agent(candidate, session)
            });
            *sessions = kept;
            sessions.push(record);
            displaced = removed;
            Ok(())
        })?;
        Ok(displaced)
    }

    fn restore_sessions(
        &self,
        expected: &AgentSession,
        records: &[AgentSession],
    ) -> Result<(), ConfigError> {
        let path = Self::path().ok_or(ConfigError::NoConfigDir)?;
        Self::update(&path, |sessions| {
            for record in records {
                let position = sessions
                    .iter()
                    .position(|candidate| candidate.id() == record.id());
                let restored = match position {
                    Some(index) => {
                        let current = &sessions[index];
                        // Leave the pin's transferred record alone if a concurrent
                        // operation re-targeted it, rather than clobbering that change.
                        if record.id() == expected.id()
                            && current.configured_key() != expected.configured_key()
                        {
                            continue;
                        }
                        // Restore the displaced identity but keep any runtime state a
                        // live provider wrote onto the current record during the write.
                        record.clone().with_runtime_state_of(current)
                    },
                    None => record.clone(),
                };
                sessions.retain(|candidate| candidate.id() != record.id());
                sessions.push(restored);
            }
            Ok(())
        })
    }

    fn retire_orphaned_configured(
        &self,
        project: &Path,
        live_keys: &[ConfiguredAgentKey],
    ) -> Result<usize, ConfigError> {
        let path = Self::path().ok_or(ConfigError::NoConfigDir)?;
        let mut retired = 0;
        Self::update(&path, |sessions| {
            for session in sessions.iter_mut() {
                let orphaned = session.project() == project
                    && session
                        .configured_key()
                        .as_ref()
                        .is_some_and(|key| !live_keys.contains(key));
                if orphaned {
                    *session = session.clone().unconfigure();
                    retired += 1;
                }
            }
            Ok(())
        })?;
        Ok(retired)
    }

    fn set_state(&self, id: &AgentSessionId, state: AgentSessionState) -> Result<(), ConfigError> {
        let path = Self::path().ok_or(ConfigError::NoConfigDir)?;
        Self::update(&path, |sessions| {
            let index = sessions
                .iter()
                .position(|session| session.id() == id)
                .ok_or_else(|| ConfigError::AgentSessionNotFound(id.clone()))?;
            let session = sessions.remove(index).with_state(state);
            sessions.push(session);
            Ok(())
        })
    }

    fn set_owner_process_id(
        &self,
        id: &AgentSessionId,
        process_id: AgentProcessId,
        process_start_token: Option<AgentProcessStartToken>,
        launch_token: Option<LaunchToken>,
    ) -> Result<(), ConfigError> {
        let path = Self::path().ok_or(ConfigError::NoConfigDir)?;
        Self::update(&path, |sessions| {
            let session = sessions
                .iter_mut()
                .find(|session| session.id() == id)
                .ok_or_else(|| ConfigError::AgentSessionNotFound(id.clone()))?;
            Self::claim_owner(session, id, process_id, process_start_token, launch_token)
        })
    }

    fn capture_native_id(
        &self,
        id: &AgentSessionId,
        provider: AgentTool,
        native_id: NativeSessionId,
        reporter_process_id: AgentProcessId,
        reported_launch_token: Option<LaunchToken>,
    ) -> Result<(), ConfigError> {
        let path = Self::path().ok_or(ConfigError::NoConfigDir)?;
        Self::update(&path, |sessions| {
            Self::assign_native_id(
                sessions,
                id,
                provider,
                native_id,
                reporter_process_id,
                reported_launch_token.clone(),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        process::AgentTool,
        value::{CommandLine, ProcessName},
    };

    /// Builds a session record for persistence tests.
    fn session() -> AgentSession {
        AgentSession::builder()
            .id(AgentSessionId::generate().unwrap())
            .name(ProcessName::try_new("Ada").unwrap())
            .tool(AgentTool::Codex)
            .project(PathBuf::from("/repo/muster.yml"))
            .launch_command(CommandLine::try_new("codex").unwrap())
            .state(AgentSessionState::Open)
            .build()
    }

    /// State mutations preserve the record and make close history durable.
    #[test]
    fn updates_session_state_and_native_identity() {
        let dir =
            std::env::temp_dir().join(format!("muster-agent-sessions-{}", uuid::Uuid::new_v4()));
        let path = dir.join(AGENT_SESSIONS_FILE);
        let original = session().with_owner_process_id(AgentProcessId::try_new(1).unwrap());

        YamlAgentSessionStore::update(&path, |sessions| {
            YamlAgentSessionStore::replace(sessions, original.clone());
            Ok(())
        })
        .unwrap();
        YamlAgentSessionStore::update(&path, |sessions| {
            let item = sessions.first_mut().unwrap();
            *item = item
                .clone()
                .with_native_id(NativeSessionId::try_new("native").unwrap())
                .with_state(AgentSessionState::Closed);
            Ok(())
        })
        .unwrap();

        let loaded = YamlAgentSessionStore::load_from(&path).unwrap();
        assert_eq!(loaded.sessions[0].id(), original.id());
        assert_eq!(
            loaded.sessions[0].native_id().as_ref().map(AsRef::as_ref),
            Some("native")
        );
        assert_eq!(*loaded.sessions[0].state(), AgentSessionState::Closed);
        fs::remove_dir_all(dir).unwrap();
    }

    /// Atomic session-state writes retain a dotfile-managed symlink and update
    /// its target rather than replacing the alias.
    #[cfg(unix)]
    #[test]
    fn writes_session_state_through_a_symlink() {
        use std::os::unix::fs::symlink;

        let dir =
            std::env::temp_dir().join(format!("muster-agent-sessions-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let target = dir.join("shared.yml");
        let link = dir.join(AGENT_SESSIONS_FILE);
        symlink(&target, &link).unwrap();
        let record = session();

        YamlAgentSessionStore::update(&link, |sessions| {
            YamlAgentSessionStore::replace(sessions, record.clone());
            Ok(())
        })
        .unwrap();

        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            YamlAgentSessionStore::load_from(&target)
                .unwrap()
                .sessions
                .len(),
            1
        );
        fs::remove_dir_all(dir).unwrap();
    }

    /// The owning provider can change conversations while another provider
    /// cannot redirect the persisted session through an inherited hook.
    #[test]
    fn updates_identity_only_for_the_owning_provider() {
        let original = session();
        let id = original.id().clone();
        let reporter = AgentProcessId::try_new(100).unwrap();
        let first = NativeSessionId::try_new("first-native").unwrap();
        let second = NativeSessionId::try_new("second-native").unwrap();
        let mismatch = NativeSessionId::try_new("mismatch-native").unwrap();
        let mut sessions = vec![original];

        YamlAgentSessionStore::assign_native_id(
            &mut sessions,
            &id,
            AgentTool::Codex,
            first,
            reporter,
            None,
        )
        .unwrap();
        // The same reporting process switches conversations.
        YamlAgentSessionStore::assign_native_id(
            &mut sessions,
            &id,
            AgentTool::Codex,
            second.clone(),
            reporter,
            None,
        )
        .unwrap();
        // A report tagged with a different provider is rejected; the session id
        // alone is trusted, but only for the provider it was launched for.
        let result = YamlAgentSessionStore::assign_native_id(
            &mut sessions,
            &id,
            AgentTool::Claude,
            mismatch,
            reporter,
            None,
        );

        assert!(matches!(
            result,
            Err(ConfigError::AgentSessionProviderMismatch {
                id: conflict,
                expected: AgentTool::Codex,
                reported: AgentTool::Claude,
            }) if conflict == id
        ));
        assert_eq!(sessions[0].native_id().as_ref(), Some(&second));
    }

    /// A report binds whatever launcher layers sit between muster and the agent -
    /// muster trusts the session id injected into the launched environment, so
    /// codex under an npx node shim (reporting a process that is not muster's
    /// direct child) is accepted.
    #[test]
    fn records_a_report_from_a_deeply_nested_launcher() {
        let original = session();
        let id = original.id().clone();
        let mut sessions = vec![original];
        let native = NativeSessionId::try_new("codex-native").unwrap();
        let deep_reporter = AgentProcessId::try_new(4242).unwrap();

        YamlAgentSessionStore::assign_native_id(
            &mut sessions,
            &id,
            AgentTool::Codex,
            native.clone(),
            deep_reporter,
            None,
        )
        .unwrap();

        assert_eq!(sessions[0].native_id().as_ref(), Some(&native));
    }

    /// A nested same-provider launch inherits the session env var but must not
    /// overwrite the managed conversation; its report is ignored (not an error), and
    /// beginning a new launch resets the reporter so a resumed process can rebind.
    #[test]
    fn a_nested_same_provider_report_is_ignored_until_relaunch() {
        // The managed reporter must be a live process so the nested report is
        // rejected while it runs (the reporter-liveness guard).
        let managed = AgentProcessId::try_new(std::process::id()).unwrap();
        let nested = AgentProcessId::try_new(1).unwrap();
        let original = session();
        let id = original.id().clone();
        let mut sessions = vec![original];
        let managed_native = NativeSessionId::try_new("managed-native").unwrap();

        YamlAgentSessionStore::assign_native_id(
            &mut sessions,
            &id,
            AgentTool::Codex,
            managed_native.clone(),
            managed,
            None,
        )
        .unwrap();
        // The nested codex reports a different conversation and is ignored.
        YamlAgentSessionStore::assign_native_id(
            &mut sessions,
            &id,
            AgentTool::Codex,
            NativeSessionId::try_new("nested-native").unwrap(),
            nested,
            None,
        )
        .unwrap();
        assert_eq!(sessions[0].native_id().as_ref(), Some(&managed_native));

        // Beginning a new launch (a new token on the owner claim) resets the
        // reporter, so the next process, reporting the new token, rebinds.
        let relaunch = LaunchToken::try_new("relaunch").unwrap();
        sessions[0] = sessions[0]
            .clone()
            .with_launch_owner(managed, None, Some(relaunch.clone()));
        let resumed = NativeSessionId::try_new("resumed-native").unwrap();
        YamlAgentSessionStore::assign_native_id(
            &mut sessions,
            &id,
            AgentTool::Codex,
            resumed.clone(),
            nested,
            Some(relaunch),
        )
        .unwrap();
        assert_eq!(sessions[0].native_id().as_ref(), Some(&resumed));
    }

    /// Once a launch records a token, a report that carries no token (a delayed
    /// capture from a pre-token launch) is rejected, so it cannot claim the reporter
    /// slot and lock out the current launch's real, differently-PID'd report.
    #[test]
    fn a_tokenless_report_is_rejected_for_a_tokenized_launch() {
        let current = LaunchToken::try_new("current-launch").unwrap();
        let stale_reporter = AgentProcessId::try_new(100).unwrap();
        let live_reporter = AgentProcessId::try_new(200).unwrap();
        let original = session().with_launch_owner(
            AgentProcessId::try_new(1).unwrap(),
            None,
            Some(current.clone()),
        );
        let id = original.id().clone();
        let mut sessions = vec![original];

        // A pre-token launch's capture carries no token and is ignored.
        YamlAgentSessionStore::assign_native_id(
            &mut sessions,
            &id,
            AgentTool::Codex,
            NativeSessionId::try_new("pre-token-native").unwrap(),
            stale_reporter,
            None,
        )
        .unwrap();
        assert!(sessions[0].native_id().is_none());
        assert!(sessions[0].native_reporter_process_id().is_none());

        // The current launch's report, carrying the token, still binds.
        let live = NativeSessionId::try_new("current-native").unwrap();
        YamlAgentSessionStore::assign_native_id(
            &mut sessions,
            &id,
            AgentTool::Codex,
            live.clone(),
            live_reporter,
            Some(current),
        )
        .unwrap();
        assert_eq!(sessions[0].native_id().as_ref(), Some(&live));
    }

    /// While the process that bound the identity is still alive, a concurrent report
    /// from a different process (a nested launch) is rejected.
    #[test]
    fn a_live_reporter_rejects_a_concurrent_different_process() {
        let token = LaunchToken::try_new("launch").unwrap();
        let live = AgentProcessId::try_new(std::process::id()).unwrap();
        let live_token = LocalProcessIdentity::start_token(live);
        assert!(
            live_token.is_some(),
            "this platform must expose start tokens"
        );
        let managed = NativeSessionId::try_new("managed-native").unwrap();
        let original = session()
            .with_launch_owner(live, None, Some(token.clone()))
            .with_reported_native_id(managed.clone(), live, live_token);
        let id = original.id().clone();
        let mut sessions = vec![original];

        YamlAgentSessionStore::assign_native_id(
            &mut sessions,
            &id,
            AgentTool::Codex,
            NativeSessionId::try_new("nested-native").unwrap(),
            AgentProcessId::try_new(1).unwrap(),
            Some(token),
        )
        .unwrap();

        assert_eq!(sessions[0].native_id().as_ref(), Some(&managed));
    }

    /// A wrapper that keeps a launch alive by restarting the provider yields a new
    /// PID under the same token. Once the process that bound the identity has exited
    /// (its recorded start token no longer matches), the replacement rebinds.
    #[test]
    fn a_replacement_rebinds_after_the_prior_reporter_exits() {
        let token = LaunchToken::try_new("launch").unwrap();
        let live = AgentProcessId::try_new(std::process::id()).unwrap();
        // A start token that cannot match the live PID's real one - modelling the
        // reporter that bound the identity having exited (or the PID being reused).
        let exited_token = AgentProcessStartToken::try_new(1).unwrap();
        assert_ne!(LocalProcessIdentity::start_token(live), Some(exited_token));
        let original = session()
            .with_launch_owner(live, None, Some(token.clone()))
            .with_reported_native_id(
                NativeSessionId::try_new("first-native").unwrap(),
                live,
                Some(exited_token),
            );
        let id = original.id().clone();
        let mut sessions = vec![original];

        let rebound = NativeSessionId::try_new("restarted-native").unwrap();
        YamlAgentSessionStore::assign_native_id(
            &mut sessions,
            &id,
            AgentTool::Codex,
            rebound.clone(),
            AgentProcessId::try_new(2).unwrap(),
            Some(token),
        )
        .unwrap();

        assert_eq!(sessions[0].native_id().as_ref(), Some(&rebound));
    }

    /// When the bound reporter is still running but its start token is unknown (an
    /// unsupported target or a metadata lookup failure), the lock is kept - a
    /// different process must not overwrite the managed conversation.
    #[test]
    fn an_unknown_start_token_keeps_the_reporter_lock_while_alive() {
        let token = LaunchToken::try_new("launch").unwrap();
        let live = AgentProcessId::try_new(std::process::id()).unwrap();
        let managed = NativeSessionId::try_new("managed-native").unwrap();
        // Bound without a start token, so liveness cannot be confirmed by token
        // even though the process is running.
        let original = session()
            .with_launch_owner(live, None, Some(token.clone()))
            .with_reported_native_id(managed.clone(), live, None);
        let id = original.id().clone();
        let mut sessions = vec![original];

        YamlAgentSessionStore::assign_native_id(
            &mut sessions,
            &id,
            AgentTool::Codex,
            NativeSessionId::try_new("other-native").unwrap(),
            AgentProcessId::try_new(1).unwrap(),
            Some(token),
        )
        .unwrap();

        assert_eq!(sessions[0].native_id().as_ref(), Some(&managed));
    }

    /// A capture left in flight by a previous launch carries that launch's token,
    /// which no longer matches the current launch, so it is ignored and cannot bind
    /// or claim the reporter slot; the current launch's own report then binds.
    #[test]
    fn a_stale_launch_report_cannot_bind_the_new_launch_identity() {
        let previous = LaunchToken::try_new("launch-a").unwrap();
        let current = LaunchToken::try_new("launch-b").unwrap();
        let stale_reporter = AgentProcessId::try_new(100).unwrap();
        let live_reporter = AgentProcessId::try_new(200).unwrap();
        let original = session().with_launch_owner(
            AgentProcessId::try_new(1).unwrap(),
            None,
            Some(current.clone()),
        );
        let id = original.id().clone();
        let mut sessions = vec![original];

        YamlAgentSessionStore::assign_native_id(
            &mut sessions,
            &id,
            AgentTool::Codex,
            NativeSessionId::try_new("stale-native").unwrap(),
            stale_reporter,
            Some(previous),
        )
        .unwrap();
        assert!(sessions[0].native_id().is_none());
        assert!(sessions[0].native_reporter_process_id().is_none());

        // The current launch's own report binds, since the stale one did not claim
        // the reporter slot.
        let live = NativeSessionId::try_new("launch-b-native").unwrap();
        YamlAgentSessionStore::assign_native_id(
            &mut sessions,
            &id,
            AgentTool::Codex,
            live.clone(),
            live_reporter,
            Some(current),
        )
        .unwrap();
        assert_eq!(sessions[0].native_id().as_ref(), Some(&live));
    }

    /// A live owner cannot be replaced by another instance claiming the same
    /// durable session.
    #[test]
    fn rejects_a_second_claim_while_the_existing_owner_is_live() {
        let owner = AgentProcessId::try_new(std::process::id()).unwrap();
        let token = LocalProcessIdentity::start_token(owner).unwrap();
        let mut record = session().with_launch_owner(owner, Some(token), None);
        let id = record.id().clone();
        let claimant = AgentProcessId::try_new(owner.into_inner().saturating_add(1)).unwrap();

        let result = YamlAgentSessionStore::claim_owner(&mut record, &id, claimant, None, None);

        assert!(matches!(
            result,
            Err(ConfigError::AgentSessionAlreadyOwned {
                id: conflict,
                owner: live_owner,
            }) if conflict == id && live_owner == owner
        ));
        assert_eq!(record.owner_process_id(), &Some(owner));
        assert_eq!(record.owner_process_start_token(), &Some(token));
    }

    /// A claim rejected because a live owner holds the session must not touch the
    /// launch token: a losing or competing launch cannot invalidate the token the
    /// owning launch's hooks carry.
    #[test]
    fn a_rejected_claim_leaves_the_launch_token_untouched() {
        let owner = AgentProcessId::try_new(std::process::id()).unwrap();
        let token = LocalProcessIdentity::start_token(owner).unwrap();
        let held = LaunchToken::try_new("owning-launch").unwrap();
        let mut record = session().with_launch_owner(owner, Some(token), Some(held.clone()));
        let id = record.id().clone();
        let claimant = AgentProcessId::try_new(owner.into_inner().saturating_add(1)).unwrap();

        let result = YamlAgentSessionStore::claim_owner(
            &mut record,
            &id,
            claimant,
            None,
            Some(LaunchToken::try_new("competing-launch").unwrap()),
        );

        assert!(matches!(
            result,
            Err(ConfigError::AgentSessionAlreadyOwned { .. })
        ));
        assert_eq!(record.launch_token(), &Some(held));
    }

    /// Session-state writes start owner-only, preserve an existing restrictive
    /// mode, and narrow any legacy mode that exposed data to other users.
    #[cfg(unix)]
    #[test]
    fn writes_session_state_with_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;

        const OWNER_READ_ONLY_MODE: u32 = 0o400;
        const LEGACY_SHARED_MODE: u32 = 0o644;
        let dir = std::env::temp_dir().join(format!("muster-agent-mode-{}", uuid::Uuid::new_v4()));
        let path = dir.join(AGENT_SESSIONS_FILE);
        let original = session();
        let write = || {
            YamlAgentSessionStore::update(&path, |sessions| {
                YamlAgentSessionStore::replace(sessions, original.clone());
                Ok(())
            })
            .unwrap();
        };

        write();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & FILE_PERMISSION_MASK,
            PRIVATE_SESSION_FILE_MODE
        );

        fs::set_permissions(&path, fs::Permissions::from_mode(OWNER_READ_ONLY_MODE)).unwrap();
        write();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & FILE_PERMISSION_MASK,
            OWNER_READ_ONLY_MODE
        );

        fs::set_permissions(&path, fs::Permissions::from_mode(LEGACY_SHARED_MODE)).unwrap();
        write();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & FILE_PERMISSION_MASK,
            PRIVATE_SESSION_FILE_MODE
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
