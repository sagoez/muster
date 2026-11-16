//! Where Claude Code keeps muster's server entries and the settings files
//! its hooks go in, as seen from one folder. muster only reads the entries;
//! the `claude` CLI alone writes them.

use std::{
    env, fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use getset::Getters;
use muster_domain::constants::MCP_CONNECTION_KEY;
use muster_os::path::home_dir;
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

    /// The settings files of every scope but `scope` that a session in this
    /// folder also reads, each named once and never `scope`'s own path.
    pub fn other_settings(&self, scope: McpScope) -> Vec<HookFile> {
        let own = canonical(self.settings(scope).path());
        let mut seen = vec![own];
        McpScope::iter()
            .filter(|other| *other != scope)
            .map(|other| self.settings(other))
            .filter(|file| {
                let path = canonical(file.path());
                let new = !seen.contains(&path);
                seen.push(path);
                new
            })
            .collect()
    }

    /// muster's server entry in `scope`, as Claude Code keeps it.
    ///
    /// # Errors
    /// Returns [`HookFileError::Read`] or [`HookFileError::Malformed`] for a
    /// servers file that cannot be read or is not JSON.
    pub fn entry(&self, scope: McpScope) -> Result<Option<Value>, HookFileError> {
        let entry = match scope {
            McpScope::User => {
                read_json(&self.servers_file)?.and_then(|config| server_in(&config).cloned())
            },
            McpScope::Local => read_json(&self.servers_file)?.and_then(|config| {
                let projects = config.get(PROJECTS_KEY)?;
                [self.cwd.clone(), canonical(&self.cwd)]
                    .iter()
                    .find_map(|folder| projects.get(folder.to_string_lossy().as_ref()))
                    .and_then(server_in)
                    .cloned()
            }),
            McpScope::Project => read_json(&self.cwd.join(PROJECT_SERVERS_FILE))?
                .and_then(|config| server_in(&config).cloned()),
        };
        Ok(entry)
    }

    /// The scopes holding a muster entry, in Claude Code's order of
    /// precedence: local, then project, then user.
    ///
    /// # Errors
    /// Returns what [`Self::entry`] returns.
    pub fn scopes_holding(&self) -> Result<Vec<McpScope>, HookFileError> {
        let mut holding = Vec::new();
        for scope in [McpScope::Local, McpScope::Project, McpScope::User] {
            if self.entry(scope)?.is_some() {
                holding.push(scope);
            }
        }
        Ok(holding)
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

/// The JSON of the file at `path`; none when it does not exist.
///
/// # Errors
/// Returns [`HookFileError::Read`] or [`HookFileError::Malformed`].
fn read_json(path: &Path) -> Result<Option<Value>, HookFileError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(HookFileError::Read {
                path: path.to_path_buf(),
                source,
            });
        },
    };
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|error| HookFileError::Malformed {
            path: path.to_path_buf(),
            message: error.to_string(),
        })
}

#[cfg(test)]
mod tests {
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
    }

    /// Run in the home folder, the project's settings file is the user's,
    /// and it is named once, never as another scope's.
    #[test]
    fn a_file_two_scopes_share_is_named_once() {
        let (config, scratch) = scratch_config();
        let home = ClaudeConfig::builder()
            .config_dir(scratch.path().join(".claude"))
            .servers_file(config.servers_file().clone())
            .cwd(scratch.path().to_path_buf())
            .build();

        let others: Vec<PathBuf> = home
            .other_settings(McpScope::User)
            .iter()
            .map(|file| file.path().clone())
            .collect();

        assert_eq!(others, [scratch
            .path()
            .join(".claude")
            .join("settings.local.json")]);
    }

    /// Entries are read from where Claude Code keeps each scope, and the
    /// scopes holding one are listed by precedence.
    #[test]
    fn entries_are_read_per_scope() {
        let (config, _scratch) = scratch_config();
        let entry = json!({"type": "stdio", "command": "/opt/muster", "args": ["mcp"]});
        let cwd = config.cwd().to_string_lossy().into_owned();
        fs::write(
            config.servers_file(),
            json!({
                "mcpServers": {"muster": entry},
                "projects": {cwd: {"mcpServers": {"muster": entry}}},
            })
            .to_string(),
        )
        .unwrap();

        assert_eq!(config.entry(McpScope::User).unwrap(), Some(entry.clone()));
        assert_eq!(config.entry(McpScope::Local).unwrap(), Some(entry));
        assert_eq!(config.entry(McpScope::Project).unwrap(), None);
        assert_eq!(config.scopes_holding().unwrap(), [
            McpScope::Local,
            McpScope::User
        ]);

        fs::write(config.servers_file(), "{").unwrap();
        assert!(matches!(
            config.entry(McpScope::User),
            Err(HookFileError::Malformed { .. })
        ));
    }
}
