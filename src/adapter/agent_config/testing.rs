//! The fixtures the agent config tests share: a scratch home holding a
//! project, muster's entry and name, and a JSON file's value.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde_json::Value;
use tempfile::TempDir;

use super::{
    DEFAULT_SERVER_NAME, agent::Agent, agent_config_error::AgentConfigError,
    agent_config_files::AgentConfigFiles, config_locations::ConfigLocations,
    config_target::ConfigTarget, entry_change::EntryChange, mcp_registry::McpRegistry,
    mcp_scope::McpScope, planned_edit::PlannedEdit, server_entry::ServerEntry,
    server_launch::ServerLaunch, server_name::McpServerName,
};
use crate::{constants::PATH_ENV, domain::value::ProjectRoot};

/// The muster binary the fixtures' entries launch.
pub(crate) const MUSTER_BINARY: &str = "/bin/muster";
/// The argument that makes it serve MCP.
pub(crate) const MCP_ARG: &str = "mcp";
/// The project folder under a scratch home.
const PROJECT_DIR: &str = "project";
/// The folder under a scratch home that stands for `PATH`.
const BIN_DIR: &str = "bin";

/// A scratch home with a project folder, and the configs under it.
pub(crate) struct Home {
    /// The scratch folder, removed when the home drops.
    _dir: TempDir,
    /// Its path with every link followed, as a project root is.
    root: PathBuf,
}

impl Home {
    /// A fresh scratch home holding `project/`.
    pub(crate) fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::create_dir(root.join(PROJECT_DIR)).unwrap();
        Self { _dir: dir, root }
    }

    /// The scratch home's folder.
    pub(crate) fn root(&self) -> &Path {
        &self.root
    }

    /// A path under the scratch home.
    pub(crate) fn at(&self, relative: &str) -> PathBuf {
        self.root.join(relative)
    }

    /// Where this home keeps each agent's user config.
    pub(crate) fn locations(&self) -> ConfigLocations {
        ConfigLocations::from_env(&self.root, |name| {
            (name == PATH_ENV).then(|| self.at(BIN_DIR).into_os_string())
        })
    }

    /// The configs of this home, launching `/bin/muster mcp`.
    pub(crate) fn files(&self) -> AgentConfigFiles {
        AgentConfigFiles::new(Some(self.locations()), launch())
    }

    /// `agent`'s config of `scope`, for the scratch project.
    pub(crate) fn target(&self, agent: Agent, scope: McpScope) -> ConfigTarget {
        ConfigTarget::builder()
            .agent(agent)
            .scope(scope)
            .root(Some(ProjectRoot::try_new(self.at(PROJECT_DIR)).unwrap()))
            .build()
    }

    /// Plans `change` to `agent`'s `scope` config and commits it.
    pub(crate) fn apply(
        &self,
        agent: Agent,
        scope: McpScope,
        change: EntryChange,
        force: bool,
    ) -> Result<PlannedEdit, AgentConfigError> {
        let files = self.files();
        let plan = files.plan(
            &[self.target(agent, scope)],
            &muster(),
            change,
            force,
            &uncancelled,
        )?;
        let edit = plan.edits()[0].clone();
        files.commit(plan, &uncancelled)?;
        Ok(edit)
    }
}

/// How the fixtures' entries launch muster: `/bin/muster mcp`.
pub(crate) fn launch() -> ServerLaunch {
    ServerLaunch::builder()
        .command(PathBuf::from(MUSTER_BINARY))
        .args(vec![MCP_ARG.to_owned()])
        .build()
}

/// The default server name.
pub(crate) fn muster() -> McpServerName {
    McpServerName::try_new(DEFAULT_SERVER_NAME).unwrap()
}

/// muster's entry, with the empty `env` Zed's carries when `with_env`.
pub(super) fn entry(with_env: bool) -> ServerEntry {
    ServerEntry::builder()
        .command(MUSTER_BINARY.to_owned())
        .args(vec![MCP_ARG.to_owned()])
        .with_env(with_env)
        .build()
}

/// The JSON file at `path`.
pub(crate) fn json_at(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
}

/// A run nobody interrupts.
pub(crate) fn uncancelled() -> bool {
    false
}
