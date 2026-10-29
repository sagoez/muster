use std::{collections::BTreeMap, ffi::OsString, path::PathBuf};

use getset::{CopyGetters, Getters};
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use crate::domain::{
    agent_session::AgentSessionId,
    process::ProcessKind,
    pty::{PtySize, SpawnHistory},
    value::{CommandLine, ProcessName},
};

/// A request to spawn one process under a PTY.
#[derive(Clone, Debug, Serialize, Deserialize, Getters, CopyGetters, TypedBuilder)]
#[serde(rename_all = "kebab-case")]
pub struct SpawnRequest {
    /// Name of the process being spawned, as its project knows it.
    #[getset(get = "pub")]
    process: ProcessName,
    /// Section the process belongs to.
    #[getset(get_copy = "pub")]
    kind: ProcessKind,
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
    #[serde(with = "environment_pairs")]
    environment: BTreeMap<OsString, OsString>,
    /// Durable session whose provider process must bind before it starts.
    #[getset(get = "pub")]
    #[builder(default)]
    agent_session_id: Option<AgentSessionId>,
    /// Initial PTY size.
    #[getset(get_copy = "pub")]
    size: PtySize,
    /// What the process's earlier runs leave to this one.
    #[getset(get = "pub")]
    #[builder(default)]
    history: SpawnHistory,
}

/// Carries the environment as a list of name and value pairs, since a name is
/// an OS string that formats such as JSON cannot use as a map key.
mod environment_pairs {
    use std::{collections::BTreeMap, ffi::OsString};

    use serde::{Deserialize, Deserializer, Serializer};

    /// Writes `environment` as a sequence of pairs.
    ///
    /// # Errors
    /// Returns the serializer's error.
    pub(super) fn serialize<S: Serializer>(
        environment: &BTreeMap<OsString, OsString>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(environment)
    }

    /// Reads a sequence of pairs back into the environment.
    ///
    /// # Errors
    /// Returns the deserializer's error.
    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<OsString, OsString>, D::Error> {
        Ok(Vec::<(OsString, OsString)>::deserialize(deserializer)?
            .into_iter()
            .collect())
    }
}
