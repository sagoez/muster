//! Codex's home folder, as `muster mcp add codex` and `remove` see it: its
//! hooks file.

use std::{env, path::PathBuf};

use getset::Getters;
use muster_os::path::home_dir;
use typed_builder::TypedBuilder;

use crate::cli::mcp::{hook_file::HookFile, hook_file_error::HookFileError, mcp_agent::McpAgent};

/// The variable that moves Codex's home folder off `~/.codex`.
pub const CODEX_HOME_ENV: &str = "CODEX_HOME";
/// Codex's home folder, in the home folder.
const CODEX_DIR: &str = ".codex";
/// Codex's hooks file.
const HOOKS_FILE: &str = "hooks.json";

/// Codex's home folder.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct CodexConfig {
    /// `$CODEX_HOME`, else `~/.codex`.
    home: PathBuf,
}

impl CodexConfig {
    /// Codex's home folder: `$CODEX_HOME` when it is set and not empty,
    /// else `.codex` in the home folder.
    ///
    /// # Errors
    /// Returns [`HookFileError::NoHome`] when neither is known.
    pub fn find() -> Result<Self, HookFileError> {
        let home = env::var_os(CODEX_HOME_ENV)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| home_dir().map(|home| home.join(CODEX_DIR)))
            .ok_or(HookFileError::NoHome(McpAgent::Codex))?;
        Ok(Self::builder().home(home).build())
    }

    /// Codex's hooks file, in the user's own config folder.
    pub fn hooks(&self) -> HookFile {
        HookFile::builder()
            .path(self.home.join(HOOKS_FILE))
            .private(true)
            .build()
    }
}
