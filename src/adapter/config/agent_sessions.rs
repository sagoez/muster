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
            AgentSessionState, NativeSessionId,
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
        wrapper_process_id: Option<AgentProcessId>,
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
        *session = session.clone().with_launch_processes(
            process_id,
            process_start_token,
            wrapper_process_id,
        );
        Ok(())
    }

    /// Replaces the current conversation identity when the owning provider or
    /// its direct wrapper child reports a new conversation.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the session is absent or the lifecycle
    /// event came from a different provider.
    fn assign_native_id(
        sessions: &mut [AgentSession],
        id: &AgentSessionId,
        provider: AgentTool,
        process_id: AgentProcessId,
        parent_process_id: Option<AgentProcessId>,
        native_id: NativeSessionId,
    ) -> Result<(), ConfigError> {
        Self::assign_native_id_with_start_token(
            sessions,
            id,
            provider,
            process_id,
            parent_process_id,
            LocalProcessIdentity::start_token(process_id),
            native_id,
        )
    }

    /// Stores `native_id` only when the lifecycle event belongs to the durable
    /// owner process identity, including its non-reusable creation token.
    ///
    /// # Errors
    /// Returns a [`ConfigError`] when the session is absent or the lifecycle
    /// event does not belong to its managed provider process.
    fn assign_native_id_with_start_token(
        sessions: &mut [AgentSession],
        id: &AgentSessionId,
        provider: AgentTool,
        process_id: AgentProcessId,
        parent_process_id: Option<AgentProcessId>,
        process_start_token: Option<AgentProcessStartToken>,
        native_id: NativeSessionId,
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
        let owns_process = session.owner_process_id().as_ref() == Some(&process_id)
            && session
                .owner_process_start_token()
                .as_ref()
                .is_none_or(|expected| process_start_token.as_ref() == Some(expected));
        let is_wrapper_handoff = session.wrapper_process_id().is_some()
            && session.wrapper_process_id().as_ref() == parent_process_id.as_ref();
        if !owns_process && !is_wrapper_handoff {
            return Err(ConfigError::AgentSessionProcessMismatch {
                id: id.clone(),
                expected: *session.owner_process_id(),
                reported: process_id,
            });
        }
        *session = session.clone().with_native_id(native_id);
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
        wrapper_process_id: Option<AgentProcessId>,
    ) -> Result<(), ConfigError> {
        let path = Self::path().ok_or(ConfigError::NoConfigDir)?;
        Self::update(&path, |sessions| {
            let session = sessions
                .iter_mut()
                .find(|session| session.id() == id)
                .ok_or_else(|| ConfigError::AgentSessionNotFound(id.clone()))?;
            Self::claim_owner(
                session,
                id,
                process_id,
                process_start_token,
                wrapper_process_id,
            )
        })
    }

    fn capture_native_id(
        &self,
        id: &AgentSessionId,
        provider: AgentTool,
        process_id: AgentProcessId,
        parent_process_id: Option<AgentProcessId>,
        native_id: NativeSessionId,
    ) -> Result<(), ConfigError> {
        let path = Self::path().ok_or(ConfigError::NoConfigDir)?;
        Self::update(&path, |sessions| {
            Self::assign_native_id(
                sessions,
                id,
                provider,
                process_id,
                parent_process_id,
                native_id,
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
        let first = NativeSessionId::try_new("first-native").unwrap();
        let second = NativeSessionId::try_new("second-native").unwrap();
        let descendant = NativeSessionId::try_new("descendant-native").unwrap();
        let mut sessions = vec![original];

        let owner = AgentProcessId::try_new(1).unwrap();
        sessions[0] = sessions[0].clone().with_owner_process_id(owner);
        YamlAgentSessionStore::assign_native_id(
            &mut sessions,
            &id,
            AgentTool::Codex,
            owner,
            None,
            first,
        )
        .unwrap();
        YamlAgentSessionStore::assign_native_id(
            &mut sessions,
            &id,
            AgentTool::Codex,
            owner,
            None,
            second.clone(),
        )
        .unwrap();
        let result = YamlAgentSessionStore::assign_native_id(
            &mut sessions,
            &id,
            AgentTool::Claude,
            owner,
            None,
            descendant,
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

    /// Same-provider descendants cannot replace their managed parent's native ID.
    #[test]
    fn rejects_same_provider_identity_from_another_process() {
        let original = session().with_owner_process_id(AgentProcessId::try_new(1).unwrap());
        let id = original.id().clone();
        let mut sessions = vec![original];
        let owner = AgentProcessId::try_new(1).unwrap();
        let descendant = AgentProcessId::try_new(2).unwrap();
        let first = NativeSessionId::try_new("parent-native").unwrap();
        let child = NativeSessionId::try_new("child-native").unwrap();

        YamlAgentSessionStore::assign_native_id(
            &mut sessions,
            &id,
            AgentTool::Codex,
            owner,
            None,
            first.clone(),
        )
        .unwrap();
        let result = YamlAgentSessionStore::assign_native_id(
            &mut sessions,
            &id,
            AgentTool::Codex,
            descendant,
            None,
            child,
        );

        assert!(matches!(
            result,
            Err(ConfigError::AgentSessionProcessMismatch {
                id: conflict,
                expected: Some(expected),
                reported,
            }) if conflict == id && expected == owner && reported == descendant
        ));
        assert_eq!(sessions[0].native_id().as_ref(), Some(&first));
    }

    /// A recycled numeric PID cannot claim a durable session with another
    /// process creation token.
    #[test]
    fn rejects_a_reused_owner_pid_with_a_different_start_token() {
        let owner = AgentProcessId::try_new(1).unwrap();
        let stored_token = AgentProcessStartToken::try_new(10).unwrap();
        let reused_token = AgentProcessStartToken::try_new(11).unwrap();
        let original = session().with_launch_processes(owner, Some(stored_token), None);
        let id = original.id().clone();
        let mut sessions = vec![original];

        let result = YamlAgentSessionStore::assign_native_id_with_start_token(
            &mut sessions,
            &id,
            AgentTool::Codex,
            owner,
            None,
            Some(reused_token),
            NativeSessionId::try_new("reused-native").unwrap(),
        );

        assert!(matches!(
            result,
            Err(ConfigError::AgentSessionProcessMismatch {
                id: conflict,
                expected: Some(expected),
                reported,
            }) if conflict == id && expected == owner && reported == owner
        ));
        assert!(sessions[0].native_id().is_none());
    }

    /// A live owner cannot be replaced by another instance claiming the same
    /// durable session.
    #[test]
    fn rejects_a_second_claim_while_the_existing_owner_is_live() {
        let owner = AgentProcessId::try_new(std::process::id()).unwrap();
        let token = LocalProcessIdentity::start_token(owner).unwrap();
        let mut record = session().with_launch_processes(owner, Some(token), None);
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

    /// A provider launched directly by the managed shell may report its own
    /// PID while retaining the shell as its immediate parent.
    #[test]
    fn accepts_identity_from_a_direct_provider_child() {
        let owner = AgentProcessId::try_new(1).unwrap();
        let provider = AgentProcessId::try_new(2).unwrap();
        let original = session().with_launch_processes(owner, None, Some(owner));
        let id = original.id().clone();
        let native = NativeSessionId::try_new("pipeline-native").unwrap();
        let mut sessions = vec![original];

        YamlAgentSessionStore::assign_native_id(
            &mut sessions,
            &id,
            AgentTool::Codex,
            provider,
            Some(owner),
            native.clone(),
        )
        .unwrap();

        assert_eq!(sessions[0].native_id().as_ref(), Some(&native));
        assert_eq!(sessions[0].owner_process_id(), &Some(owner));

        let switched = NativeSessionId::try_new("resumed-native").unwrap();
        YamlAgentSessionStore::assign_native_id(
            &mut sessions,
            &id,
            AgentTool::Codex,
            provider,
            Some(owner),
            switched.clone(),
        )
        .unwrap();
        assert_eq!(sessions[0].native_id().as_ref(), Some(&switched));

        let nested = AgentProcessId::try_new(3).unwrap();
        let result = YamlAgentSessionStore::assign_native_id(
            &mut sessions,
            &id,
            AgentTool::Codex,
            nested,
            Some(provider),
            NativeSessionId::try_new("nested-native").unwrap(),
        );
        assert!(matches!(
            result,
            Err(ConfigError::AgentSessionProcessMismatch { .. })
        ));
        assert_eq!(sessions[0].native_id().as_ref(), Some(&switched));
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
