//! muster's server entries in Claude Code's servers files, as one command
//! reads them: each file parsed at most once, and only when a scope it holds
//! is asked about.

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::cli::mcp::{
    agent_file::read_optional, claude_config::ClaudeConfig, hook_file_error::HookFileError,
    mcp_scope::McpScope,
};

/// One servers file as read: its JSON, none when absent, or `None` when it
/// could not be read.
type ReadFile = (PathBuf, Option<Option<Value>>);

/// The servers files one command has read, and the failures not yet told.
pub struct ServerEntries<'a> {
    /// Where the files are.
    config: &'a ClaudeConfig,
    /// Each file read so far.
    read: Vec<ReadFile>,
    /// The failures of files that could not be read, until taken.
    unread: Vec<HookFileError>,
}

impl<'a> ServerEntries<'a> {
    /// Nothing read yet of `config`'s servers files.
    pub fn new(config: &'a ClaudeConfig) -> Self {
        Self {
            config,
            read: Vec::new(),
            unread: Vec::new(),
        }
    }

    /// muster's entry in `scope`: `Some(None)` when the scope holds none,
    /// `None` when its servers file could not be read, whose failure
    /// [`Self::take_unread`] then gives once.
    pub fn entry(&mut self, scope: McpScope) -> Option<Option<Value>> {
        let path = self.config.servers_path(scope);
        if !self.read.iter().any(|(seen, _)| *seen == path) {
            let json = read_json(&path)
                .map_err(|error| self.unread.push(error))
                .ok();
            self.read.push((path.clone(), json));
        }
        let config = self.config;
        self.read
            .iter()
            .find(|(seen, _)| *seen == path)
            .and_then(|(_, json)| json.as_ref())
            .map(|json| {
                json.as_ref()
                    .and_then(|json| config.server_of(scope, json).cloned())
            })
    }

    /// The failures of the files read since last asked, each once.
    pub fn take_unread(&mut self) -> Vec<HookFileError> {
        std::mem::take(&mut self.unread)
    }
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
    use std::fs;

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    /// Entries are read from where Claude Code keeps each scope, each file
    /// parsed once, so a file changed after it was read is not read again,
    /// and one that cannot be read leaves only its own scopes unknown.
    #[test]
    fn each_servers_file_is_read_once() {
        let scratch = TempDir::new().unwrap();
        let cwd = scratch.path().join("shop");
        fs::create_dir_all(&cwd).unwrap();
        let config = ClaudeConfig::builder()
            .config_dir(scratch.path().join("claude"))
            .servers_file(scratch.path().join("claude.json"))
            .cwd(cwd.clone())
            .build();
        let entry = json!({"type": "stdio", "command": "/opt/muster", "args": ["mcp"]});
        let folder = cwd.to_string_lossy().into_owned();
        fs::write(
            config.servers_file(),
            json!({
                "mcpServers": {"muster": entry},
                "projects": {folder: {"mcpServers": {"muster": entry}}},
            })
            .to_string(),
        )
        .unwrap();
        fs::write(cwd.join(".mcp.json"), "{").unwrap();
        let mut entries = ServerEntries::new(&config);

        assert_eq!(entries.entry(McpScope::Local), Some(Some(entry.clone())));
        fs::write(config.servers_file(), "{}").unwrap();
        assert_eq!(
            entries.entry(McpScope::User),
            Some(Some(entry)),
            "parsed once, with the local scope"
        );
        assert_eq!(entries.entry(McpScope::Project), None);
        assert_eq!(entries.entry(McpScope::Project), None);
        assert_eq!(entries.take_unread().len(), 1, "told once");
        assert!(entries.take_unread().is_empty());
    }
}
