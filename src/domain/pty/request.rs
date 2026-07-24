use std::{collections::BTreeMap, ffi::OsString, path::PathBuf};

use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use crate::domain::{agent_session::AgentSessionId, pty::PtySize, value::CommandLine};

/// A request to spawn one process under a PTY.
#[derive(Clone, Debug, Getters, CopyGetters, TypedBuilder)]
pub struct SpawnRequest {
    /// Command to run, or the user's login shell when absent.
    #[getset(get = "pub")]
    #[builder(default)]
    command: Option<CommandLine>,
    /// Directory to launch in; inherits the parent's cwd when absent.
    #[getset(get = "pub")]
    #[builder(default)]
    working_dir: Option<PathBuf>,
    /// Project config path exported to the process, letting the `muster` CLI
    /// target the current project without a flag. Absent leaves it unset.
    #[getset(get = "pub")]
    #[builder(default)]
    project: Option<PathBuf>,
    /// Additional environment exported only to this child process.
    #[getset(get = "pub")]
    #[builder(default)]
    environment: BTreeMap<OsString, OsString>,
    /// Durable session whose provider process must bind before it starts.
    #[getset(get = "pub")]
    #[builder(default)]
    agent_session_id: Option<AgentSessionId>,
    /// Initial PTY size.
    #[getset(get_copy = "pub")]
    size: PtySize,
}
