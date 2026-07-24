use std::path::PathBuf;

use thiserror::Error;

use crate::domain::{
    agent_session::{AgentProcessId, AgentSessionId},
    process::{AgentTool, ProcessKind},
    value::{ProcessName, ProjectName},
};

/// Errors from workspace configuration and project registry operations.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// The configuration file could not be read from disk.
    #[error("could not read config file {path}: {source}")]
    Read {
        /// Path that failed to load.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// The configuration file could not be written to disk.
    #[error("could not write config file {path}: {source}")]
    Write {
        /// Path that failed to write.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// No writable config directory could be located on this platform.
    #[error("no config directory is available")]
    NoConfigDir,
    /// The session-state file uses a schema newer than this binary understands.
    #[error("unsupported agent-session state version {0}")]
    UnsupportedAgentSessionVersion(u8),
    /// A lifecycle update referenced a session no longer present in state.
    #[error("agent session '{0}' is not present in session state")]
    AgentSessionNotFound(AgentSessionId),
    /// A lifecycle event came from a provider that does not own the session.
    #[error(
        "agent session '{id}' belongs to {expected}, but received a lifecycle event from {reported}"
    )]
    AgentSessionProviderMismatch {
        /// Session whose provider rejected the lifecycle event.
        id: AgentSessionId,
        /// Provider recorded when the session was created.
        expected: AgentTool,
        /// Provider that emitted the lifecycle event.
        reported: AgentTool,
    },
    /// A lifecycle event came from a descendant instead of the managed provider process.
    #[error(
        "agent session '{id}' belongs to process {expected:?}, but received a lifecycle event from process {reported}"
    )]
    AgentSessionProcessMismatch {
        /// Session whose provider process rejected the lifecycle event.
        id: AgentSessionId,
        /// Process currently owning the managed agent session.
        expected: Option<AgentProcessId>,
        /// Process that emitted the lifecycle event.
        reported: AgentProcessId,
    },
    /// A concurrent Muster instance already owns the managed agent session.
    #[error("agent session '{id}' is already owned by live process {owner}")]
    AgentSessionAlreadyOwned {
        /// Session whose ownership claim was rejected.
        id: AgentSessionId,
        /// Live provider process retaining ownership.
        owner: AgentProcessId,
    },
    /// A legacy registry entry has no stable location outside its original
    /// launch directory.
    #[error(
        "registered project '{name}' uses unsupported relative config path {path:?}; edit or remove it in projects.yml"
    )]
    RelativeProjectConfig {
        /// Registered project whose config path is ambiguous.
        name: ProjectName,
        /// Relative config path stored by an earlier version.
        path: PathBuf,
    },
    /// The configuration was not valid YAML or violated the schema.
    #[error("could not parse config: {0}")]
    Parse(#[from] serde_yaml_ng::Error),
    /// A non-command process configured command-only graceful shutdown.
    #[error("{kind} process '{name}' configures stop, which is only valid for commands")]
    InvalidStopPolicy {
        /// Section containing the invalid process.
        kind: ProcessKind,
        /// Name of the invalid process.
        name: ProcessName,
    },
}
