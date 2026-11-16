//! Codex's home folder, as `muster mcp add codex` and `remove` see it: its
//! hooks file, and its `config.toml`, where an existing muster server is
//! pointed at this binary in place, every other key of it kept, so its
//! environment never passes on a command line.

use std::{
    env, fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use getset::Getters;
use muster_domain::constants::{MCP_CONNECTION_KEY, MCP_SUBCOMMAND};
use muster_os::path::home_dir;
use toml_edit::{Array, DocumentMut, Item, TomlError, Value};
use typed_builder::TypedBuilder;

use crate::cli::{
    mcp::{
        agent_keys::{ARGS_KEY, COMMAND_KEY},
        codex_entry::CodexEntry,
        hook_file::HookFile,
        hook_file_error::HookFileError,
        mcp_agent::McpAgent,
    },
    transfer::text_file::write_atomically,
};

/// The variable that moves Codex's home folder off `~/.codex`.
pub const CODEX_HOME_ENV: &str = "CODEX_HOME";
/// Codex's home folder, in the home folder.
const CODEX_DIR: &str = ".codex";
/// Codex's hooks file.
const HOOKS_FILE: &str = "hooks.json";
/// Codex's config file, holding its servers.
const CONFIG_FILE: &str = "config.toml";
/// The table of Codex's servers, by name.
const SERVERS_TABLE: &str = "mcp_servers";

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

    /// Codex's hooks file.
    pub fn hooks(&self) -> HookFile {
        HookFile::builder().path(self.home.join(HOOKS_FILE)).build()
    }

    /// The config file's path.
    pub fn config_path(&self) -> PathBuf {
        self.home.join(CONFIG_FILE)
    }

    /// The config file's text; none when it does not exist.
    ///
    /// # Errors
    /// Returns [`HookFileError::Read`] when it cannot be read.
    pub fn config_text(&self) -> Result<Option<String>, HookFileError> {
        let path = self.config_path();
        match fs::read_to_string(&path) {
            Ok(text) => Ok(Some(text)),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(source) => Err(HookFileError::Read { path, source }),
        }
    }

    /// Writes `text` as the config file, atomically, through a link to the
    /// file it names.
    ///
    /// # Errors
    /// Returns [`HookFileError::Write`] when it cannot be written.
    pub fn write_config(&self, text: &str) -> Result<(), HookFileError> {
        let path = self.config_path();
        let target = fs::canonicalize(&path).unwrap_or_else(|_| path.clone());
        write_atomically(&target, text).map_err(|source| HookFileError::Write { path, source })
    }
}

/// What `text`, Codex's config at `path` or none when absent, holds for
/// muster against an entry running `binary` as muster's server: the edited
/// text pointing an existing entry at `binary`, its arguments kept when
/// they run muster's server and every other key as it was.
///
/// # Errors
/// Returns [`HookFileError::Malformed`] for a config that is not TOML, and
/// [`HookFileError::NotCommand`] for a muster entry that runs no command,
/// which muster did not make.
pub fn codex_entry(
    text: Option<&str>,
    binary: &str,
    path: &Path,
) -> Result<CodexEntry, HookFileError> {
    let Some(text) = text else {
        return Ok(CodexEntry::Absent);
    };
    let mut config: DocumentMut =
        text.parse()
            .map_err(|error: TomlError| HookFileError::Malformed {
                path: path.to_path_buf(),
                message: error.to_string(),
            })?;
    let Some(entry) = config
        .get_mut(SERVERS_TABLE)
        .and_then(|servers| servers.get_mut(MCP_CONNECTION_KEY))
    else {
        return Ok(CodexEntry::Absent);
    };
    let not_command = || HookFileError::NotCommand {
        path: path.to_path_buf(),
    };
    let entry = entry.as_table_like_mut().ok_or_else(not_command)?;
    let command = entry
        .get(COMMAND_KEY)
        .and_then(Item::as_str)
        .ok_or_else(not_command)?;
    let serves = entry
        .get(ARGS_KEY)
        .and_then(Item::as_array)
        .and_then(|args| args.get(0))
        .and_then(|first| first.as_str())
        .is_some_and(|first| first == MCP_SUBCOMMAND);
    if command == binary && serves {
        return Ok(CodexEntry::Current);
    }
    let command = kept_decor(entry.get(COMMAND_KEY), Value::from(binary));
    entry.insert(COMMAND_KEY, command);
    if !serves {
        let args = kept_decor(
            entry.get(ARGS_KEY),
            Value::from(Array::from_iter([MCP_SUBCOMMAND])),
        );
        entry.insert(ARGS_KEY, args);
    }
    Ok(CodexEntry::Edited(config.to_string()))
}

/// `new` as an item with the spacing and comment around `old`, the value it
/// replaces, so the line keeps its look.
fn kept_decor(old: Option<&Item>, mut new: Value) -> Item {
    if let Some(old) = old.and_then(Item::as_value) {
        *new.decor_mut() = old.decor().clone();
    }
    Item::Value(new)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A config Codex wrote, with muster's server customised and another's.
    const CONFIG: &str = r#"model = "o3" # mine

[mcp_servers.other]
command = "other"

[mcp_servers.muster]
command = "/old/muster" # pinned
args = ["mcp", "--as", "reviewer"]
startup_timeout_sec = 20
env = { MUSTER_AGENT = "reviewer", TOKEN = "secret" }
"#;

    /// Pointing an entry at a new binary changes its command alone, every
    /// other key and line kept, and an entry already current is left
    /// alone.
    #[test]
    fn an_entry_is_pointed_at_the_binary_in_place() {
        let path = Path::new("/c/config.toml");

        let CodexEntry::Edited(text) = codex_entry(Some(CONFIG), "/new/muster", path).unwrap()
        else {
            panic!("an old command is edited");
        };

        assert_eq!(text, CONFIG.replace("\"/old/muster\"", "\"/new/muster\""));
        assert_eq!(
            codex_entry(Some(&text), "/new/muster", path).unwrap(),
            CodexEntry::Current
        );
    }

    /// No config, or none naming muster, means Codex registers it; a config
    /// that is not TOML, or a muster entry running no command, is refused.
    #[test]
    fn an_absent_or_foreign_entry_is_told_apart() {
        let path = Path::new("/c/config.toml");

        assert_eq!(codex_entry(None, "/m", path).unwrap(), CodexEntry::Absent);
        assert_eq!(
            codex_entry(Some("model = \"o3\"\n"), "/m", path).unwrap(),
            CodexEntry::Absent
        );
        assert!(matches!(
            codex_entry(Some("[mcp_servers"), "/m", path),
            Err(HookFileError::Malformed { .. })
        ));
        assert!(matches!(
            codex_entry(
                Some("[mcp_servers.muster]\nurl = \"http://x\"\n"),
                "/m",
                path
            ),
            Err(HookFileError::NotCommand { .. })
        ));
    }

    /// An entry whose arguments run something else is given muster's.
    #[test]
    fn foreign_arguments_are_replaced() {
        let path = Path::new("/c/config.toml");
        let config = "[mcp_servers.muster]\ncommand = \"/m\"\nargs = [\"serve\"]\n";

        let CodexEntry::Edited(text) = codex_entry(Some(config), "/m", path).unwrap() else {
            panic!("the arguments are fixed");
        };

        assert!(text.contains("args = [\"mcp\"]"), "{text}");
    }
}
