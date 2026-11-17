//! Where Claude Code keeps muster's server entries and the settings files
//! its hooks go in, as seen from one folder. muster only reads the entries;
//! the `claude` CLI alone writes them.

use std::{
    env, fs,
    path::{Path, PathBuf},
};

use getset::Getters;
use muster_domain::constants::MCP_CONNECTION_KEY;
use muster_os::path::home_dir;
use serde_json::Value;
use strum::IntoEnumIterator;
use typed_builder::TypedBuilder;

use crate::cli::mcp::{
    agent_file::read_optional,
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

/// Whether each scope asked about holds a muster entry: `Some(true)` or
/// `Some(false)`, or `None` when its servers file could not be read.
pub type Holding = Vec<(McpScope, Option<bool>)>;

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

    /// muster's server entry in `scope`, as Claude Code keeps it.
    ///
    /// # Errors
    /// Returns [`HookFileError::Read`] or [`HookFileError::Malformed`] for a
    /// servers file that cannot be read or is not JSON.
    pub fn entry(&self, scope: McpScope) -> Result<Option<Value>, HookFileError> {
        Ok(read_json(&self.servers_path(scope))?
            .and_then(|config| self.server_of(scope, &config).cloned()))
    }

    /// Whether each of `scopes`, in order, holds a muster entry, each
    /// servers file read once, with the failure of each file that could not
    /// be read; that file's scopes are then unknown.
    pub fn holding(&self, scopes: &[McpScope]) -> (Holding, Vec<HookFileError>) {
        let mut read: Vec<(PathBuf, Option<Option<Value>>)> = Vec::new();
        let mut unread = Vec::new();
        let mut holding = Vec::new();
        for &scope in scopes {
            let path = self.servers_path(scope);
            if !read.iter().any(|(seen, _)| *seen == path) {
                let config = read_json(&path).map_err(|error| unread.push(error)).ok();
                read.push((path.clone(), config));
            }
            let held = read
                .iter()
                .find(|(seen, _)| *seen == path)
                .and_then(|(_, config)| config.as_ref())
                .map(|config| {
                    config
                        .as_ref()
                        .and_then(|config| self.server_of(scope, config))
                        .is_some()
                });
            holding.push((scope, held));
        }
        (holding, unread)
    }

    /// The file holding `scope`'s servers.
    fn servers_path(&self, scope: McpScope) -> PathBuf {
        match scope {
            McpScope::User | McpScope::Local => self.servers_file.clone(),
            McpScope::Project => self.cwd.join(PROJECT_SERVERS_FILE),
        }
    }

    /// muster's entry for `scope` in `config`, its servers file.
    fn server_of<'a>(&self, scope: McpScope, config: &'a Value) -> Option<&'a Value> {
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

/// The file `path` names with every link resolved, as far as the nearest
/// part of it that exists, so two paths to one file compare equal even
/// before the file or its folder is made.
fn resolved(path: &Path) -> PathBuf {
    path.ancestors()
        .find_map(|known| {
            let real = fs::canonicalize(known).ok()?;
            let rest = path.strip_prefix(known).ok()?;
            Some(real.join(rest))
        })
        .unwrap_or_else(|| path.to_path_buf())
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
    let Some(text) = read_optional(path)? else {
        return Ok(None);
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

    /// Entries are read from where Claude Code keeps each scope, and a file
    /// that cannot be read leaves only its own scopes unknown.
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
        let all: Vec<McpScope> = McpScope::iter().collect();

        assert_eq!(config.entry(McpScope::User).unwrap(), Some(entry.clone()));
        assert_eq!(config.entry(McpScope::Local).unwrap(), Some(entry));
        assert_eq!(config.entry(McpScope::Project).unwrap(), None);
        let (holding, unread) = config.holding(&all);
        assert_eq!(holding, [
            (McpScope::Local, Some(true)),
            (McpScope::Project, Some(false)),
            (McpScope::User, Some(true)),
        ]);
        assert!(unread.is_empty());

        fs::write(config.servers_file(), "{").unwrap();
        fs::write(config.cwd().join(".mcp.json"), r#"{"mcpServers": {}}"#).unwrap();
        assert!(matches!(
            config.entry(McpScope::User),
            Err(HookFileError::Malformed { .. })
        ));
        let (holding, unread) = config.holding(&all);
        assert_eq!(holding, [
            (McpScope::Local, None),
            (McpScope::Project, Some(false)),
            (McpScope::User, None),
        ]);
        assert_eq!(unread.len(), 1, "the one servers file, read once");
    }
}
