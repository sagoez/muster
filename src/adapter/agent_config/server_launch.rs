use std::path::PathBuf;

use getset::Getters;
use typed_builder::TypedBuilder;

use super::{agent_config_error::AgentConfigError, mcp_scope::McpScope, server_entry::ServerEntry};
use crate::constants::APP_NAME;

/// How an agent starts muster's MCP server: the program and its arguments.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
pub struct ServerLaunch {
    /// The muster binary as the person would type it, for their own
    /// configs.
    command: PathBuf,
    /// The arguments that make it serve MCP.
    args: Vec<String>,
}

impl ServerLaunch {
    /// muster's entry for a config of `scope`: a project's file, shared
    /// with whoever clones the project, names `muster` for each person's
    /// own `PATH` to find; a user's own config names this binary. With
    /// `with_env`, the entry carries an empty `env`.
    ///
    /// # Errors
    /// Returns [`AgentConfigError::CommandNotUtf8`] for a binary path a
    /// config file cannot hold as text.
    pub(super) fn entry(
        &self,
        scope: McpScope,
        with_env: bool,
    ) -> Result<ServerEntry, AgentConfigError> {
        let command = if scope == McpScope::Project {
            APP_NAME
        } else {
            self.command
                .to_str()
                .ok_or_else(|| AgentConfigError::CommandNotUtf8(self.command.clone()))?
        };
        Ok(ServerEntry::builder()
            .command(command.to_owned())
            .args(self.args.clone())
            .with_env(with_env)
            .build())
    }
}
