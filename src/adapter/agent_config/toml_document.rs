//! An edit of one server entry in a Codex `config.toml`, made on its
//! formatting-preserving document so every other table, comment and blank
//! line stays as written.

use std::path::Path;

use toml_edit::{Array, DocumentMut, Item, Table, TableLike, Value, value};

use super::{
    agent_config_error::AgentConfigError,
    edit_outcome::EditOutcome,
    edited::Edited,
    entry_change::EntryChange,
    server_entry::{ARGS_KEY, COMMAND_KEY, ServerEntry},
    server_name::McpServerName,
};

/// The table Codex reads its MCP servers from.
const SERVERS_TABLE: &str = "mcp_servers";

/// `change` applied to the entry `name` in `text`, the Codex config at
/// `path`, or in an empty one when there is none.
///
/// # Errors
/// Returns [`AgentConfigError::Unparseable`] or
/// [`AgentConfigError::NotATable`] for a file muster cannot edit safely, and
/// [`AgentConfigError::Taken`] or [`AgentConfigError::NotOurs`] for a
/// different entry under `name` without `force`.
pub(super) fn edit_toml(
    text: Option<&str>,
    path: &Path,
    name: &McpServerName,
    entry: &ServerEntry,
    change: EntryChange,
    force: bool,
) -> Result<Edited, AgentConfigError> {
    let mut document = text
        .unwrap_or_default()
        .parse::<DocumentMut>()
        .map_err(|error| AgentConfigError::Unparseable {
            path: path.to_path_buf(),
            reason: error.message().to_owned(),
        })?;
    let not_a_table = || AgentConfigError::NotATable {
        path: path.to_path_buf(),
        key: SERVERS_TABLE.to_owned(),
    };
    if change == EntryChange::Add && !document.contains_key(SERVERS_TABLE) {
        let mut servers = Table::new();
        servers.set_implicit(true);
        document.insert(SERVERS_TABLE, Item::Table(servers));
    }
    let outcome = match document.get_mut(SERVERS_TABLE) {
        None => EditOutcome::Absent,
        Some(item) => {
            let servers = item.as_table_like_mut().ok_or_else(not_a_table)?;
            let existing = servers.get(name.as_ref()).map(|item| holds(item, entry));
            match (change, existing) {
                (EntryChange::Add, None) => {
                    servers.insert(name.as_ref(), entry_table(entry));
                    EditOutcome::Added
                },
                (EntryChange::Add, Some(true)) => EditOutcome::AlreadyAdded,
                (EntryChange::Add, Some(false)) if force => {
                    servers.insert(name.as_ref(), entry_table(entry));
                    EditOutcome::Replaced
                },
                (EntryChange::Add, Some(false)) => {
                    return Err(AgentConfigError::Taken {
                        path: path.to_path_buf(),
                        name: name.clone(),
                    });
                },
                (EntryChange::Remove, None) => EditOutcome::Absent,
                (EntryChange::Remove, Some(ours)) if ours || force => {
                    servers.remove(name.as_ref());
                    EditOutcome::Removed
                },
                (EntryChange::Remove, Some(_)) => {
                    return Err(AgentConfigError::NotOurs {
                        path: path.to_path_buf(),
                        name: name.clone(),
                    });
                },
            }
        },
    };
    let text = outcome.writes().then(|| document.to_string());
    Ok(Edited::builder().outcome(outcome).text(text).build())
}

/// `entry` as the table Codex reads: its command and arguments.
fn entry_table(entry: &ServerEntry) -> Item {
    let mut table = Table::new();
    table.insert(COMMAND_KEY, value(entry.command().as_str()));
    table.insert(
        ARGS_KEY,
        value(entry.args().iter().map(String::as_str).collect::<Array>()),
    );
    Item::Table(table)
}

/// Whether `item`, an existing entry, is `entry`.
fn holds(item: &Item, entry: &ServerEntry) -> bool {
    let Some(table) = item.as_table_like() else {
        return false;
    };
    entry.matches(
        string_at(table, COMMAND_KEY),
        table
            .get(ARGS_KEY)
            .and_then(Item::as_array)
            .map(|args| args.iter().map(Value::as_str).collect()),
    )
}

/// The string under `key` in `table`, if it holds one.
fn string_at<'a>(table: &'a dyn TableLike, key: &str) -> Option<&'a str> {
    table.get(key).and_then(Item::as_str)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// The path every fixture is said to live at.
    fn file() -> PathBuf {
        PathBuf::from("/h/.codex/config.toml")
    }

    /// muster's entry.
    fn entry() -> ServerEntry {
        ServerEntry::builder()
            .command("/bin/muster".to_owned())
            .args(vec!["mcp".to_owned()])
            .with_env(false)
            .build()
    }

    /// The default server name.
    fn muster() -> McpServerName {
        McpServerName::try_new("muster").unwrap()
    }

    /// A Codex config with comments, settings and another server.
    const CODEX_CONFIG: &str = r#"# my codex settings
model = "o3"  # the one I like

[mcp_servers.other]
# keep this server
command = "other-mcp"
args = ["--flag"]

[profiles.fast]
model = "o4-mini"
"#;

    /// Adding keeps every comment, setting and server, writes a table of
    /// its own, and a second add writes nothing.
    #[test]
    fn a_codex_config_keeps_everything_else() {
        let added = edit_toml(
            Some(CODEX_CONFIG),
            &file(),
            &muster(),
            &entry(),
            EntryChange::Add,
            false,
        )
        .unwrap();
        let text = added.text().clone().unwrap();

        assert_eq!(added.outcome(), EditOutcome::Added);
        assert!(
            text.contains(
                "args = [\"--flag\"]\n\n[mcp_servers.muster]\ncommand = \"/bin/muster\"\nargs = [\"mcp\"]\n\n[profiles.fast]"
            ),
            "the entry joins the other servers:\n{text}"
        );
        for line in CODEX_CONFIG.lines() {
            assert!(text.contains(line), "{line} lost from:\n{text}");
        }

        let again = edit_toml(
            Some(&text),
            &file(),
            &muster(),
            &entry(),
            EntryChange::Add,
            false,
        )
        .unwrap();
        assert_eq!(again.outcome(), EditOutcome::AlreadyAdded);
        assert!(again.text().is_none());

        let removed = edit_toml(
            Some(&text),
            &file(),
            &muster(),
            &entry(),
            EntryChange::Remove,
            false,
        )
        .unwrap();
        assert_eq!(removed.outcome(), EditOutcome::Removed);
        assert_eq!(removed.text().as_deref(), Some(CODEX_CONFIG));
    }

    /// A missing config gets the entry alone, with no empty parent table.
    #[test]
    fn a_missing_config_gets_the_entry_alone() {
        let added = edit_toml(None, &file(), &muster(), &entry(), EntryChange::Add, false).unwrap();
        assert_eq!(
            added.text().as_deref(),
            Some("[mcp_servers.muster]\ncommand = \"/bin/muster\"\nargs = [\"mcp\"]\n")
        );
        let removed = edit_toml(
            None,
            &file(),
            &muster(),
            &entry(),
            EntryChange::Remove,
            false,
        )
        .unwrap();
        assert_eq!(removed.outcome(), EditOutcome::Absent);
    }

    /// An inline table of servers takes the entry inline.
    #[test]
    fn an_inline_table_of_servers_stays_inline() {
        let text = "mcp_servers = { other = { command = \"x\" } }\n";
        let added = edit_toml(
            Some(text),
            &file(),
            &muster(),
            &entry(),
            EntryChange::Add,
            false,
        )
        .unwrap();
        let written = added.text().clone().unwrap();
        let reread = written.parse::<DocumentMut>().unwrap();
        assert_eq!(
            reread[SERVERS_TABLE]["muster"][COMMAND_KEY].as_str(),
            Some("/bin/muster")
        );
        assert_eq!(
            reread[SERVERS_TABLE]["other"][COMMAND_KEY].as_str(),
            Some("x")
        );
    }

    /// A different entry under the name needs force; a broken file or a
    /// servers key that is no table is refused.
    #[test]
    fn a_different_entry_or_a_broken_file_is_refused() {
        let theirs = "[mcp_servers.muster]\ncommand = \"/old/muster\"\nargs = [\"mcp\"]\n";
        assert!(matches!(
            edit_toml(
                Some(theirs),
                &file(),
                &muster(),
                &entry(),
                EntryChange::Add,
                false
            ),
            Err(AgentConfigError::Taken { .. })
        ));
        assert!(matches!(
            edit_toml(
                Some(theirs),
                &file(),
                &muster(),
                &entry(),
                EntryChange::Remove,
                false
            ),
            Err(AgentConfigError::NotOurs { .. })
        ));
        let forced = edit_toml(
            Some(theirs),
            &file(),
            &muster(),
            &entry(),
            EntryChange::Add,
            true,
        )
        .unwrap();
        assert_eq!(forced.outcome(), EditOutcome::Replaced);
        assert!(forced.text().as_ref().unwrap().contains("\"/bin/muster\""));

        assert!(matches!(
            edit_toml(
                Some("model = "),
                &file(),
                &muster(),
                &entry(),
                EntryChange::Add,
                false
            ),
            Err(AgentConfigError::Unparseable { .. })
        ));
        assert!(matches!(
            edit_toml(
                Some("mcp_servers = 3\n"),
                &file(),
                &muster(),
                &entry(),
                EntryChange::Add,
                false
            ),
            Err(AgentConfigError::NotATable { .. })
        ));
    }
}
