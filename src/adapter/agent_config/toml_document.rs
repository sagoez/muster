//! An edit of one server entry in a Codex `config.toml`, made on its
//! formatting-preserving document so every other table, comment and blank
//! line stays as written.

use std::path::Path;

use toml_edit::{Array, DocumentMut, Item, Table, Value, value};

use super::{
    agent_config_error::AgentConfigError,
    edit_outcome::EditOutcome,
    edited::Edited,
    entry_action::EntryAction,
    entry_change::EntryChange,
    entry_match::EntryMatch,
    server_entry::{ARGS_KEY, COMMAND_KEY, ENABLED_KEY, ServerEntry, turned_off},
    server_name::McpServerName,
};

/// The table Codex reads its MCP servers from.
const SERVERS_TABLE: &str = "mcp_servers";

/// `change` applied to the entry `name` in `text`, the Codex config at
/// `path`, or in an empty one when there is none. A removal that empties
/// the table of servers takes it out too.
///
/// # Errors
/// Returns [`AgentConfigError::Unparseable`] or
/// [`AgentConfigError::NotATable`] for a file muster cannot edit safely, and
/// [`AgentConfigError::Taken`] or [`AgentConfigError::NotOurs`] for another
/// server's entry under `name` without `force`.
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
    if change == EntryChange::Add && !document.contains_key(SERVERS_TABLE) {
        let mut servers = Table::new();
        servers.set_implicit(true);
        document.insert(SERVERS_TABLE, Item::Table(servers));
    }
    let Some(item) = document.get_mut(SERVERS_TABLE) else {
        return Ok(Edited::builder()
            .outcome(EditOutcome::Absent)
            .text(None)
            .build());
    };
    let servers = item
        .as_table_like_mut()
        .ok_or_else(|| AgentConfigError::NotATable {
            path: path.to_path_buf(),
            key: SERVERS_TABLE.to_owned(),
        })?;
    let judged = servers.get(name.as_ref()).map(|item| judge(item, entry));
    let action = EntryAction::decide(
        judged.map(|(matched, _)| matched),
        judged.is_some_and(|(_, off)| off),
        change,
        force,
        path,
        name,
    )?;
    match action {
        EntryAction::Insert | EntryAction::Replace => {
            servers.insert(name.as_ref(), entry_table(entry));
        },
        EntryAction::Update | EntryAction::Enable => {
            if let Some(existing) = servers
                .get_mut(name.as_ref())
                .and_then(Item::as_table_like_mut)
            {
                existing.insert(COMMAND_KEY, value(entry.command().as_str()));
                if action == EntryAction::Enable {
                    existing.remove(ENABLED_KEY);
                }
            }
        },
        EntryAction::Remove => {
            servers.remove(name.as_ref());
            if servers.is_empty() {
                document.remove(SERVERS_TABLE);
            }
        },
        EntryAction::Keep(_) => {},
    }
    let outcome = action.outcome();
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

/// What `item`, an existing entry, is to muster, and whether it is turned
/// off.
fn judge(item: &Item, entry: &ServerEntry) -> (EntryMatch, bool) {
    let Some(table) = item.as_table_like() else {
        return (EntryMatch::Foreign, false);
    };
    let matched = entry.judge(
        table.get(COMMAND_KEY).and_then(Item::as_str),
        table
            .get(ARGS_KEY)
            .and_then(Item::as_array)
            .map(|args| args.iter().map(Value::as_str).collect()),
    );
    let enabled = table.get(ENABLED_KEY).and_then(Item::as_bool);
    (matched, turned_off(enabled))
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::adapter::agent_config::testing::{MUSTER_BINARY, entry, muster};

    /// The path every fixture is said to live at.
    fn file() -> PathBuf {
        PathBuf::from("/h/.codex/config.toml")
    }

    /// `change` made to muster's entry in `text`.
    fn edited(
        text: Option<&str>,
        change: EntryChange,
        force: bool,
    ) -> Result<Edited, AgentConfigError> {
        edit_toml(text, &file(), &muster(), &entry(false), change, force)
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
        let added = edited(Some(CODEX_CONFIG), EntryChange::Add, false).unwrap();
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

        let again = edited(Some(&text), EntryChange::Add, false).unwrap();
        assert_eq!(again.outcome(), EditOutcome::AlreadyAdded);
        assert!(again.text().is_none());

        let removed = edited(Some(&text), EntryChange::Remove, false).unwrap();
        assert_eq!(removed.outcome(), EditOutcome::Removed);
        assert_eq!(removed.text().as_deref(), Some(CODEX_CONFIG));
    }

    /// A missing config gets the entry alone, with no empty parent table,
    /// and removing it leaves no empty table of servers behind.
    #[test]
    fn a_missing_config_gets_the_entry_alone() {
        let added = edited(None, EntryChange::Add, false).unwrap();
        assert_eq!(
            added.text().as_deref(),
            Some("[mcp_servers.muster]\ncommand = \"/bin/muster\"\nargs = [\"mcp\"]\n")
        );
        let removed = edited(added.text().as_deref(), EntryChange::Remove, false).unwrap();
        assert_eq!(removed.text().as_deref(), Some(""));
        let absent = edited(None, EntryChange::Remove, false).unwrap();
        assert_eq!(absent.outcome(), EditOutcome::Absent);
    }

    /// An inline table of servers takes the entry inline.
    #[test]
    fn an_inline_table_of_servers_stays_inline() {
        let text = "mcp_servers = { other = { command = \"x\" } }\n";
        let added = edited(Some(text), EntryChange::Add, false).unwrap();
        let written = added.text().clone().unwrap();
        let reread = written.parse::<DocumentMut>().unwrap();
        assert_eq!(
            reread[SERVERS_TABLE]["muster"][COMMAND_KEY].as_str(),
            Some(MUSTER_BINARY)
        );
        assert_eq!(
            reread[SERVERS_TABLE]["other"][COMMAND_KEY].as_str(),
            Some("x")
        );
    }

    /// muster's entry from another install's path is updated in place, its
    /// other keys kept; a turned-off one is reported, and turned back on in
    /// place by force, its flag cleared and every other key kept. A
    /// `disabled` key, which Codex does not read, turns nothing off.
    #[test]
    fn an_older_or_turned_off_entry_of_musters() {
        let older = "[mcp_servers.muster]\ncommand = \"/old/bin/muster\"\nargs = [\"mcp\"]\nstartup_timeout_sec = 20\n";
        let updated = edited(Some(older), EntryChange::Add, false).unwrap();
        assert_eq!(updated.outcome(), EditOutcome::Updated);
        assert_eq!(
            updated.text().as_deref(),
            Some(
                "[mcp_servers.muster]\ncommand = \"/bin/muster\"\nargs = [\"mcp\"]\nstartup_timeout_sec = 20\n"
            )
        );

        let off = "[mcp_servers.muster]\ncommand = \"/old/bin/muster\"\nargs = [\"mcp\"]\nenabled = false\nstartup_timeout_sec = 20\n";
        let kept = edited(Some(off), EntryChange::Add, false).unwrap();
        assert_eq!(kept.outcome(), EditOutcome::Disabled);
        assert!(kept.text().is_none());
        let forced = edited(Some(off), EntryChange::Add, true).unwrap();
        assert_eq!(forced.outcome(), EditOutcome::Enabled);
        assert_eq!(
            forced.text().as_deref(),
            Some(
                "[mcp_servers.muster]\ncommand = \"/bin/muster\"\nargs = [\"mcp\"]\nstartup_timeout_sec = 20\n"
            )
        );

        let unread =
            "[mcp_servers.muster]\ncommand = \"/bin/muster\"\nargs = [\"mcp\"]\ndisabled = true\n";
        let same = edited(Some(unread), EntryChange::Add, false).unwrap();
        assert_eq!(same.outcome(), EditOutcome::AlreadyAdded);
    }

    /// Another server under the name needs force; a broken file or a
    /// servers key that is no table is refused.
    #[test]
    fn another_servers_entry_or_a_broken_file_is_refused() {
        let theirs = "[mcp_servers.muster]\ncommand = \"/opt/other-tool\"\nargs = [\"mcp\"]\n";
        assert!(matches!(
            edited(Some(theirs), EntryChange::Add, false),
            Err(AgentConfigError::Taken { .. })
        ));
        assert!(matches!(
            edited(Some(theirs), EntryChange::Remove, false),
            Err(AgentConfigError::NotOurs { .. })
        ));
        let forced = edited(Some(theirs), EntryChange::Add, true).unwrap();
        assert_eq!(forced.outcome(), EditOutcome::Replaced);
        assert!(forced.text().as_ref().unwrap().contains("\"/bin/muster\""));

        assert!(matches!(
            edited(Some("model = "), EntryChange::Add, false),
            Err(AgentConfigError::Unparseable { .. })
        ));
        assert!(matches!(
            edited(Some("mcp_servers = 3\n"), EntryChange::Add, false),
            Err(AgentConfigError::NotATable { .. })
        ));
    }
}
