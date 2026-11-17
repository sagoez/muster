//! Where Claude Code keeps muster's server entries and the settings files
//! its hooks go in, as seen from one folder. muster only reads the entries;
//! the `claude` CLI alone writes them.

use std::{
    env, fs,
    path::{Path, PathBuf},
};

use getset::Getters;
use muster_domain::constants::MCP_CONNECTION_KEY;
use muster_os::path::{home_dir, resolved};
use serde_json::Value;
use strum::IntoEnumIterator;
use typed_builder::TypedBuilder;

use crate::cli::mcp::{
    agent_keys::{MCP_SERVERS_KEY, PROJECTS_KEY},
    hook_file::HookFile,
    hook_file_error::HookFileError,
    mcp_agent::McpAgent,
    mcp_scope::McpScope,
};

/// The variable that moves Claude Code's config folder off `~/.claude`.
pub const CLAUDE_CONFIG_ENV: &str = "CLAUDE_CONFIG_DIR";
/// Claude Code's config folder, in the home folder and in a project.
const CLAUDE_DIR: &str = ".claude";
/// The file holding the user's and each folder's local servers.
const SERVERS_FILE: &str = ".claude.json";
/// The file holding a project's shared servers.
const PROJECT_SERVERS_FILE: &str = ".mcp.json";
/// Claude Code's settings file, shared in a project or the user's own.
const SETTINGS_FILE: &str = "settings.json";
/// Claude Code's settings file a project keeps out of version control.
const LOCAL_SETTINGS_FILE: &str = "settings.local.json";

/// Claude Code's files as seen from the folder muster runs in.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ClaudeConfig {
    /// Claude Code's config folder: `$CLAUDE_CONFIG_DIR`, else `~/.claude`.
    config_dir: PathBuf,
    /// The file holding user and local servers: `.claude.json` in
    /// `$CLAUDE_CONFIG_DIR`, else in the home folder.
    servers_file: PathBuf,
    /// The folder muster runs in, whose project and local scopes these are.
    cwd: PathBuf,
}

impl ClaudeConfig {
    /// Claude Code's files for `cwd`, under `$CLAUDE_CONFIG_DIR` when it is
    /// set and not empty, else the home folder.
    ///
    /// # Errors
    /// Returns [`HookFileError::NoHome`] when neither is known.
    pub fn find(cwd: PathBuf) -> Result<Self, HookFileError> {
        let (config_dir, servers_file) = match env::var_os(CLAUDE_CONFIG_ENV)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
        {
            Some(dir) => (dir.clone(), dir.join(SERVERS_FILE)),
            None => {
                let home = home_dir().ok_or(HookFileError::NoHome(McpAgent::Claude))?;
                (home.join(CLAUDE_DIR), home.join(SERVERS_FILE))
            },
        };
        Ok(Self::builder()
            .config_dir(config_dir)
            .servers_file(servers_file)
            .cwd(cwd)
            .build())
    }

    /// The settings file of `scope`, which its hooks go in.
    pub fn settings(&self, scope: McpScope) -> HookFile {
        let path = match scope {
            McpScope::User => self.config_dir.join(SETTINGS_FILE),
            McpScope::Project => self.cwd.join(CLAUDE_DIR).join(SETTINGS_FILE),
            McpScope::Local => self.cwd.join(CLAUDE_DIR).join(LOCAL_SETTINGS_FILE),
        };
        HookFile::builder().path(path).build()
    }

    /// The other scopes whose settings file is the same file as `scope`'s,
    /// as the user's and the project's are when muster runs in the home
    /// folder.
    pub fn sharing(&self, scope: McpScope) -> Vec<McpScope> {
        let own = resolved(self.settings(scope).path());
        McpScope::iter()
            .filter(|other| *other != scope && resolved(self.settings(*other).path()) == own)
            .collect()
    }

    /// The file holding `scope`'s servers.
    pub fn servers_path(&self, scope: McpScope) -> PathBuf {
        match scope {
            McpScope::User | McpScope::Local => self.servers_file.clone(),
            McpScope::Project => self.cwd.join(PROJECT_SERVERS_FILE),
        }
    }

    /// muster's entry for `scope` in `config`, its servers file.
    pub fn server_of<'a>(&self, scope: McpScope, config: &'a Value) -> Option<&'a Value> {
        match scope {
            McpScope::User | McpScope::Project => server_in(config),
            McpScope::Local => {
                let projects = config.get(PROJECTS_KEY)?;
                [self.cwd.clone(), canonical(&self.cwd)]
                    .iter()
                    .find_map(|folder| projects.get(folder.to_string_lossy().as_ref()))
                    .and_then(server_in)
            },
        }
    }
}

/// `path` with its links resolved, or as given when it does not exist.
fn canonical(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// muster's entry among the servers of `config`.
fn server_in(config: &Value) -> Option<&Value> {
    config.get(MCP_SERVERS_KEY)?.get(MCP_CONNECTION_KEY)
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    /// A config for a `shop` folder in a fresh scratch folder.
    fn scratch_config() -> (ClaudeConfig, TempDir) {
        let scratch = TempDir::new().unwrap();
        let cwd = scratch.path().join("shop");
        fs::create_dir_all(&cwd).unwrap();
        let config = ClaudeConfig::builder()
            .config_dir(scratch.path().join("claude"))
            .servers_file(scratch.path().join("claude.json"))
            .cwd(cwd)
            .build();
        (config, scratch)
    }

    /// Each scope's settings file is the one Claude Code reads for it.
    #[test]
    fn each_scope_has_its_settings_file() {
        let (config, scratch) = scratch_config();

        assert_eq!(
            config.settings(McpScope::User).path(),
            &scratch.path().join("claude").join("settings.json")
        );
        assert_eq!(
            config.settings(McpScope::Project).path(),
            &config.cwd().join(".claude").join("settings.json")
        );
        assert_eq!(
            config.settings(McpScope::Local).path(),
            &config.cwd().join(".claude").join("settings.local.json")
        );
        assert!(config.sharing(McpScope::User).is_empty());
    }

    /// Run in the home folder, even reached through a link, the user's and
    /// the project's settings are one file, and each scope sees the other
    /// sharing it.
    #[test]
    fn the_home_folder_shares_the_users_settings() {
        let scratch = TempDir::new().unwrap();
        let home = scratch.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let linked = scratch.path().join("linked");
        symlink(&home, &linked).unwrap();
        let config = ClaudeConfig::builder()
            .config_dir(home.join(".claude"))
            .servers_file(home.join(".claude.json"))
            .cwd(linked)
            .build();

        assert_eq!(config.sharing(McpScope::Project), [McpScope::User]);
        assert_eq!(config.sharing(McpScope::User), [McpScope::Project]);
        assert!(config.sharing(McpScope::Local).is_empty());
    }

    /// Each scope's entry is found where Claude Code keeps it in its file.
    #[test]
    fn each_scope_has_its_entry() {
        let (config, _scratch) = scratch_config();
        let entry = json!({"type": "stdio", "command": "/opt/muster", "args": ["mcp"]});
        let cwd = config.cwd().to_string_lossy().into_owned();
        let servers = json!({
            "mcpServers": {"muster": entry},
            "projects": {cwd: {"mcpServers": {}}},
        });

        assert_eq!(config.server_of(McpScope::User, &servers), Some(&entry));
        assert_eq!(config.server_of(McpScope::Local, &servers), None);
        assert_eq!(
            config.servers_path(McpScope::Project),
            config.cwd().join(".mcp.json")
        );
    }
}
