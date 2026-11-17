//! Where Claude Code keeps muster's server entries and the settings files
//! its hooks go in, as seen from one folder. muster only reads the entries;
//! the `claude` CLI alone writes them.

use std::{
    env,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use getset::Getters;
use muster_domain::constants::MCP_CONNECTION_KEY;
use muster_os::path::{home_dir, resolved};
use serde_json::Value;
use strum::IntoEnumIterator;
use typed_builder::TypedBuilder;

use crate::cli::mcp::{
    agent_file_error::AgentFileError,
    agent_keys::{MCP_SERVERS_KEY, PROJECTS_KEY},
    hook_file::HookFile,
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
/// The git binary, asked which repository a folder is in.
const GIT: &str = "git";
/// Asks git for the repository's shared git folder as an absolute path:
/// the main working tree's `.git`, for a linked worktree too.
const COMMON_DIR_QUERY: [&str; 3] = ["rev-parse", "--path-format=absolute", "--git-common-dir"];
/// A repository's git folder, inside its main working tree.
const GIT_DIR: &str = ".git";
/// Ends git's answer.
const LINE_END: char = '\n';

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
    /// The folder Claude Code keys this folder's local servers by: the main
    /// working tree of the repository it is in, else the folder itself;
    /// none when that cannot be told.
    local_key: Option<PathBuf>,
}

impl ClaudeConfig {
    /// Claude Code's files for `cwd`, under `$CLAUDE_CONFIG_DIR` when it is
    /// set and not empty, else the home folder.
    ///
    /// # Errors
    /// Returns [`AgentFileError::NoHome`] when neither is known.
    pub fn find(cwd: PathBuf) -> Result<Self, AgentFileError> {
        let (config_dir, servers_file) = match env::var_os(CLAUDE_CONFIG_ENV)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
        {
            Some(dir) => (dir.clone(), dir.join(SERVERS_FILE)),
            None => {
                let home = home_dir().ok_or(AgentFileError::NoHome(McpAgent::Claude))?;
                (home.join(CLAUDE_DIR), home.join(SERVERS_FILE))
            },
        };
        Ok(Self::builder()
            .config_dir(config_dir)
            .servers_file(servers_file)
            .local_key(local_key(&cwd))
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
        HookFile::builder()
            .path(path)
            .private(scope == McpScope::User)
            .build()
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

    /// muster's entry for `scope` in `config`, its servers file: a local
    /// one under the folder Claude Code keys it by, however that folder is
    /// spelled there.
    pub fn server_of<'a>(&self, scope: McpScope, config: &'a Value) -> Option<&'a Value> {
        match scope {
            McpScope::User | McpScope::Project => server_in(config),
            McpScope::Local => {
                let key = resolved(self.local_key.as_ref()?);
                config
                    .get(PROJECTS_KEY)?
                    .as_object()?
                    .iter()
                    .find(|(folder, _)| resolved(Path::new(folder)) == key)
                    .and_then(|(_, project)| server_in(project))
            },
        }
    }
}

/// The folder Claude Code keys `cwd`'s local servers by: the main working
/// tree of the repository holding `cwd`, as git names it, else `cwd`
/// itself. None when git cannot say and a `.git` in `cwd` or above says
/// `cwd` may be in a repository after all.
fn local_key(cwd: &Path) -> Option<PathBuf> {
    let output = Command::new(GIT)
        .args(COMMON_DIR_QUERY)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .filter(|output| output.status.success());
    let Some(output) = output else {
        return cwd
            .ancestors()
            .all(|folder| !folder.join(GIT_DIR).exists())
            .then(|| cwd.to_path_buf());
    };
    let common = PathBuf::from(
        String::from_utf8(output.stdout)
            .ok()?
            .trim_end_matches(LINE_END),
    );
    if common.file_name()? != GIT_DIR {
        return None;
    }
    common.parent().map(Path::to_path_buf)
}

/// muster's entry among the servers of `config`.
fn server_in(config: &Value) -> Option<&Value> {
    config.get(MCP_SERVERS_KEY)?.get(MCP_CONNECTION_KEY)
}

#[cfg(test)]
mod tests {
    use std::{fs, os::unix::fs::symlink};

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
            .local_key(Some(cwd.clone()))
            .cwd(cwd)
            .build();
        (config, scratch)
    }

    /// Runs git with `args` in `folder`, as a test sets up a repository.
    fn git(folder: &Path, args: &[&str]) {
        let done = Command::new(GIT)
            .args(args)
            .current_dir(folder)
            .output()
            .unwrap();
        assert!(done.status.success(), "{done:?}");
    }

    /// Claude Code keys a folder's local servers by the main working tree
    /// of its repository, from a subfolder and from a linked worktree
    /// alike, and by the folder itself outside any repository; muster
    /// finds the entry there, however the key is spelled.
    #[test]
    fn a_local_entry_is_found_where_claude_code_keys_it() {
        let scratch = TempDir::new().unwrap();
        let repo = scratch.path().join("repo");
        let deep = repo.join("sub").join("deep");
        fs::create_dir_all(&deep).unwrap();
        git(&repo, &["init", "-q"]);
        git(&repo, &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "x",
        ]);
        let worktree = scratch.path().join("wt");
        git(&repo, &[
            "worktree",
            "add",
            "-q",
            worktree.to_str().unwrap(),
        ]);
        let plain = scratch.path().join("plain");
        fs::create_dir_all(&plain).unwrap();

        assert_eq!(
            local_key(&deep).map(|key| resolved(&key)),
            Some(resolved(&repo))
        );
        assert_eq!(
            local_key(&worktree).map(|key| resolved(&key)),
            Some(resolved(&repo))
        );
        assert_eq!(local_key(&plain), Some(plain.clone()));

        let entry = json!({"type": "stdio", "command": "/opt/muster", "args": ["mcp"]});
        let config = ClaudeConfig::builder()
            .config_dir(scratch.path().join("claude"))
            .servers_file(scratch.path().join("claude.json"))
            .local_key(local_key(&deep))
            .cwd(deep.clone())
            .build();
        let servers = json!({"projects": {
            deep.to_string_lossy(): {"mcpServers": {}},
            repo.to_string_lossy(): {"mcpServers": {"muster": entry}},
        }});
        assert_eq!(config.server_of(McpScope::Local, &servers), Some(&entry));
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
            .local_key(Some(linked.clone()))
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
