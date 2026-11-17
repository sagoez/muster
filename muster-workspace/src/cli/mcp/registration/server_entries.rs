//! muster's server entries in Claude Code's servers files, as one command
//! reads them: each file parsed at most once, and only when a scope it holds
//! is asked about.

use std::{
    collections::HashMap,
    mem,
    path::{Path, PathBuf},
};

use getset::Getters;
use serde_json::Value;
use typed_builder::TypedBuilder;

use crate::cli::mcp::{
    agent_file::read_optional, claude_config::ClaudeConfig, entry_state::EntryState,
    hook_file_error::HookFileError, mcp_scope::McpScope, servers_file::ServersFile,
};

/// The servers files one command has read, and the failures not yet told;
/// built with nothing read yet.
#[derive(Debug, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct ServerEntries<'a> {
    /// Where the files are.
    config: &'a ClaudeConfig,
    /// Each file read so far, by its path.
    #[builder(default, setter(skip))]
    read: HashMap<PathBuf, ServersFile>,
    /// The failures of files that could not be read, until taken.
    #[builder(default, setter(skip))]
    unread: Vec<HookFileError>,
}

impl ServerEntries<'_> {
    /// What `scope`'s servers file says of muster's entry, reading the file
    /// the first time one of its scopes is asked about; the failure of a
    /// file that cannot be read [`Self::take_unread`] then gives once.
    pub fn entry(&mut self, scope: McpScope) -> EntryState {
        let path = self.config.servers_path(scope);
        if !self.read.contains_key(&path) {
            let file = match read_json(&path) {
                Ok(Some(json)) => ServersFile::Parsed(json),
                Ok(None) => ServersFile::Missing,
                Err(error) => {
                    self.unread.push(error);
                    ServersFile::Unreadable
                },
            };
            self.read.insert(path.clone(), file);
        }
        match self.read.get(&path) {
            Some(ServersFile::Parsed(json)) => self
                .config
                .server_of(scope, json)
                .cloned()
                .map_or(EntryState::Absent, EntryState::Held),
            Some(ServersFile::Missing) => EntryState::Absent,
            Some(ServersFile::Unreadable) | None => EntryState::Unknown,
        }
    }

    /// The failures of the files read since last asked, each once.
    pub fn take_unread(&mut self) -> Vec<HookFileError> {
        mem::take(&mut self.unread)
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
    /// parsed once, so a file changed after it was read is not read again;
    /// a scope with no entry, or no file, is absent, and one whose file
    /// cannot be read is unknown.
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
            json!({"projects": {folder: {"mcpServers": {"muster": entry}}}}).to_string(),
        )
        .unwrap();
        let mut entries = ServerEntries::builder().config(&config).build();

        assert_eq!(entries.entry(McpScope::Project), EntryState::Absent);
        fs::write(cwd.join(".mcp.json"), "{").unwrap();
        assert_eq!(
            entries.entry(McpScope::Project),
            EntryState::Absent,
            "read once"
        );
        assert_eq!(
            entries.entry(McpScope::Local),
            EntryState::Held(entry.clone())
        );
        fs::write(config.servers_file(), "{").unwrap();
        assert_eq!(entries.entry(McpScope::User), EntryState::Absent);
        assert!(entries.take_unread().is_empty());

        let mut fresh = ServerEntries::builder().config(&config).build();
        assert_eq!(fresh.entry(McpScope::Project), EntryState::Unknown);
        assert_eq!(fresh.entry(McpScope::User), EntryState::Unknown);
        assert_eq!(fresh.take_unread().len(), 2, "each file told once");
        assert_eq!(fresh.entry(McpScope::Local), EntryState::Unknown);
        assert!(fresh.take_unread().is_empty());
    }
}
