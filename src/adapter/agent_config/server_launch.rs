use std::path::PathBuf;

use getset::Getters;
use typed_builder::TypedBuilder;

use super::agent_config_error::AgentConfigError;

/// How an agent starts muster's MCP server: the program and its arguments.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
pub struct ServerLaunch {
    /// The muster binary, absolute so the entry works off `PATH`; read
    /// through [`Self::command_text`].
    command: PathBuf,
    /// The arguments that make it serve MCP.
    #[getset(get = "pub")]
    args: Vec<String>,
}

impl ServerLaunch {
    /// The command as the text a config file holds.
    ///
    /// # Errors
    /// Returns [`AgentConfigError::CommandNotUtf8`] for a path a config file
    /// cannot hold as text.
    pub fn command_text(&self) -> Result<&str, AgentConfigError> {
        self.command
            .to_str()
            .ok_or_else(|| AgentConfigError::CommandNotUtf8(self.command.clone()))
    }
}
