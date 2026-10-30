//! An edit of one server entry in a JSON or JSONC file, made on its concrete
//! syntax tree so every other key, comment and blank line stays as written.

use std::path::Path;

use getset::{CopyGetters, Getters};
use jsonc_parser::{
    ParseOptions,
    cst::{CstInputValue, CstNode, CstObject, CstRootNode},
};
use serde::de::IgnoredAny;
use typed_builder::TypedBuilder;

use super::{
    agent_config_error::AgentConfigError, edit_outcome::EditOutcome, edited::Edited,
    entry_change::EntryChange, server_entry::ServerEntry, server_name::McpServerName,
};

/// The text a file muster creates starts from.
const EMPTY_OBJECT: &str = "{}\n";
/// The line break a file muster writes ends with.
const LINE_BREAK: char = '\n';

/// How strictly a file is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum JsonDialect {
    /// Plain JSON, as Claude Code reads it: a comment or trailing comma
    /// makes the file one muster leaves alone.
    Strict,
    /// JSON with comments and trailing commas, as Zed reads its settings.
    WithComments,
}

/// Where in a JSON file an entry lives and how strictly the file is read.
#[derive(Clone, Debug, Getters, CopyGetters, TypedBuilder)]
pub(super) struct JsonPlace<'a> {
    /// The file, for messages.
    #[getset(get_copy = "pub(super)")]
    path: &'a Path,
    /// The keys of the objects leading to the table of servers, outermost
    /// first; the last is the table itself.
    #[getset(get = "pub(super)")]
    table: Vec<String>,
    /// How strictly the file is read.
    #[getset(get_copy = "pub(super)")]
    dialect: JsonDialect,
}

/// `change` applied to the entry `name` in `text`, the file at `place`, or
/// in an empty file when there is none.
///
/// # Errors
/// Returns [`AgentConfigError::Unparseable`] or
/// [`AgentConfigError::NotATable`] for a file muster cannot edit safely, and
/// [`AgentConfigError::Taken`] or [`AgentConfigError::NotOurs`] for a
/// different entry under `name` without `force`.
pub(super) fn edit_json(
    text: Option<&str>,
    place: &JsonPlace<'_>,
    name: &McpServerName,
    entry: &ServerEntry,
    change: EntryChange,
    force: bool,
) -> Result<Edited, AgentConfigError> {
    let source = text
        .filter(|text| !text.trim().is_empty())
        .unwrap_or(EMPTY_OBJECT);
    let unparseable = |reason: String| AgentConfigError::Unparseable {
        path: place.path().to_path_buf(),
        reason,
    };
    // The syntax tree takes comments whatever its options say, so strict
    // JSON is checked by a strict parser first.
    if place.dialect() == JsonDialect::Strict {
        serde_json::from_str::<IgnoredAny>(source)
            .map_err(|error| unparseable(error.to_string()))?;
    }
    let root = CstRootNode::parse(source, &ParseOptions::default())
        .map_err(|error| unparseable(error.to_string()))?;
    let Some(top) = root.object_value() else {
        return Err(not_a_table(place, "(the top level)"));
    };
    let outcome = match change {
        EntryChange::Add => {
            let table = table_of(&top, place, true)?.ok_or_else(|| not_a_table(place, ""))?;
            add(&table, place, name, entry, force)?
        },
        EntryChange::Remove => match table_of(&top, place, false)? {
            Some(table) => remove(&table, place, name, entry, force)?,
            None => EditOutcome::Absent,
        },
    };
    let text = outcome.writes().then(|| {
        let mut written = root.to_string();
        if !written.ends_with(LINE_BREAK) {
            written.push(LINE_BREAK);
        }
        written
    });
    Ok(Edited::builder().outcome(outcome).text(text).build())
}

/// The table of servers under `top`, made with the objects leading to it
/// when `create`, or none when it is absent and not made.
///
/// # Errors
/// Returns [`AgentConfigError::NotATable`] when a key on the way holds
/// something other than an object.
fn table_of(
    top: &CstObject,
    place: &JsonPlace<'_>,
    create: bool,
) -> Result<Option<CstObject>, AgentConfigError> {
    let mut object = top.clone();
    for key in place.table() {
        object = match object.get(key) {
            Some(prop) => prop.object_value().ok_or_else(|| not_a_table(place, key))?,
            None if create => object
                .append(key, CstInputValue::Object(Vec::new()))
                .object_value()
                .ok_or_else(|| not_a_table(place, key))?,
            None => return Ok(None),
        };
    }
    Ok(Some(object))
}

/// Adds `entry` under `name` to `table`.
///
/// # Errors
/// Returns [`AgentConfigError::Taken`] for a different entry under `name`
/// without `force`.
fn add(
    table: &CstObject,
    place: &JsonPlace<'_>,
    name: &McpServerName,
    entry: &ServerEntry,
    force: bool,
) -> Result<EditOutcome, AgentConfigError> {
    let Some(existing) = table.get(name.as_ref()) else {
        table.append(name.as_ref(), entry.json_input());
        return Ok(EditOutcome::Added);
    };
    if holds(existing.value().as_ref(), entry) {
        return Ok(EditOutcome::AlreadyAdded);
    }
    if !force {
        return Err(AgentConfigError::Taken {
            path: place.path().to_path_buf(),
            name: name.clone(),
        });
    }
    existing.set_value(entry.json_input());
    Ok(EditOutcome::Replaced)
}

/// Removes the entry under `name` from `table`.
///
/// # Errors
/// Returns [`AgentConfigError::NotOurs`] for a different entry under `name`
/// without `force`.
fn remove(
    table: &CstObject,
    place: &JsonPlace<'_>,
    name: &McpServerName,
    entry: &ServerEntry,
    force: bool,
) -> Result<EditOutcome, AgentConfigError> {
    let Some(existing) = table.get(name.as_ref()) else {
        return Ok(EditOutcome::Absent);
    };
    if !force && !holds(existing.value().as_ref(), entry) {
        return Err(AgentConfigError::NotOurs {
            path: place.path().to_path_buf(),
            name: name.clone(),
        });
    }
    existing.remove();
    Ok(EditOutcome::Removed)
}

/// Whether `value`, an existing entry, is `entry`.
fn holds(value: Option<&CstNode>, entry: &ServerEntry) -> bool {
    value
        .and_then(CstNode::to_serde_value)
        .is_some_and(|value| entry.matches_json(&value))
}

/// The refusal of `key` in the file at `place` as not an object.
fn not_a_table(place: &JsonPlace<'_>, key: &str) -> AgentConfigError {
    AgentConfigError::NotATable {
        path: place.path().to_path_buf(),
        key: key.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::{Value, json};

    use super::*;

    /// The path every fixture is said to live at.
    fn file() -> PathBuf {
        PathBuf::from("/cfg/settings.json")
    }

    /// The place `table` names in a file of `dialect`.
    fn place<'a>(path: &'a Path, table: &[&str], dialect: JsonDialect) -> JsonPlace<'a> {
        JsonPlace::builder()
            .path(path)
            .table(table.iter().map(ToString::to_string).collect())
            .dialect(dialect)
            .build()
    }

    /// muster's entry, with `env` as Zed writes it when `with_env`.
    fn entry(with_env: bool) -> ServerEntry {
        ServerEntry::builder()
            .command("/bin/muster".to_owned())
            .args(vec!["mcp".to_owned()])
            .with_env(with_env)
            .build()
    }

    /// The default server name.
    fn muster() -> McpServerName {
        McpServerName::try_new("muster").unwrap()
    }

    /// `text` as plain JSON.
    fn parsed(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    /// Zed settings with comments, a trailing comma and another server.
    const ZED_SETTINGS: &str = r#"// Zed settings
{
  // the theme I like
  "theme": "One Dark",
  "context_servers": {
    /* keep this one */
    "other": { "command": "other-mcp", "args": [] },
  },
}
"#;

    /// Adding to Zed's settings keeps every comment, the other server and
    /// the trailing commas, and writes the documented entry.
    #[test]
    fn zed_settings_keep_their_comments() {
        let path = file();
        let at = place(&path, &["context_servers"], JsonDialect::WithComments);
        let edited = edit_json(
            Some(ZED_SETTINGS),
            &at,
            &muster(),
            &entry(true),
            EntryChange::Add,
            false,
        )
        .unwrap();
        let text = edited.text().clone().unwrap();

        assert_eq!(edited.outcome(), EditOutcome::Added);
        assert!(
            text.contains(
                "    \"other\": { \"command\": \"other-mcp\", \"args\": [] },\n    \"muster\": {\n      \"command\": \"/bin/muster\",\n      \"args\": [\"mcp\"],\n      \"env\": {},\n    },\n  },\n}\n"
            ),
            "the entry follows the file's indent and trailing commas:\n{text}"
        );
        for kept in [
            "// Zed settings",
            "// the theme I like",
            "/* keep this one */",
            "\"other\": { \"command\": \"other-mcp\", \"args\": [] },",
        ] {
            assert!(text.contains(kept), "{kept} lost from:\n{text}");
        }
        let root = CstRootNode::parse(&text, &ParseOptions::default()).unwrap();
        let value = root.value().unwrap().to_serde_value().unwrap();
        assert_eq!(
            value["context_servers"]["muster"],
            json!({"command": "/bin/muster", "args": ["mcp"], "env": {}})
        );
        assert_eq!(value["theme"], "One Dark");

        let again = edit_json(
            Some(&text),
            &at,
            &muster(),
            &entry(true),
            EntryChange::Add,
            false,
        )
        .unwrap();
        assert_eq!(again.outcome(), EditOutcome::AlreadyAdded);
        assert!(again.text().is_none(), "an identical entry writes nothing");

        let removed = edit_json(
            Some(&text),
            &at,
            &muster(),
            &entry(true),
            EntryChange::Remove,
            false,
        )
        .unwrap();
        let after = removed.text().clone().unwrap();
        assert_eq!(removed.outcome(), EditOutcome::Removed);
        assert!(
            after.contains("/* keep this one */") && after.contains("other-mcp"),
            "{after}"
        );
        assert!(!after.contains("/bin/muster"), "{after}");
    }

    /// A Claude Code file gains the entry beside the servers and keys it
    /// already held, at the nested place of a local entry too.
    #[test]
    fn claude_files_keep_their_other_servers_and_keys() {
        let path = file();
        let original = r#"{
  "numStartups": 12,
  "mcpServers": {
    "other": {
      "command": "other-mcp",
      "args": []
    }
  },
  "projects": {
    "/work/shop": {
      "allowedTools": []
    }
  }
}
"#;
        let user = place(&path, &["mcpServers"], JsonDialect::Strict);
        let added = edit_json(
            Some(original),
            &user,
            &muster(),
            &entry(false),
            EntryChange::Add,
            false,
        )
        .unwrap();
        let value = parsed(added.text().as_ref().unwrap());
        assert_eq!(
            value["mcpServers"]["muster"],
            json!({"command": "/bin/muster", "args": ["mcp"]})
        );
        assert_eq!(value["mcpServers"]["other"]["command"], "other-mcp");
        assert_eq!(value["numStartups"], 12);
        assert!(
            added
                .text()
                .as_ref()
                .unwrap()
                .starts_with("{\n  \"numStartups\": 12,\n")
        );

        let local = place(
            &path,
            &["projects", "/work/shop", "mcpServers"],
            JsonDialect::Strict,
        );
        let added = edit_json(
            Some(original),
            &local,
            &muster(),
            &entry(false),
            EntryChange::Add,
            false,
        )
        .unwrap();
        let value = parsed(added.text().as_ref().unwrap());
        assert_eq!(
            value["projects"]["/work/shop"]["mcpServers"]["muster"]["args"],
            json!(["mcp"])
        );
        assert_eq!(value["projects"]["/work/shop"]["allowedTools"], json!([]));
        assert!(
            value["mcpServers"].get("muster").is_none(),
            "a local entry stays in its project"
        );
    }

    /// A missing or empty file becomes one holding just the entry.
    #[test]
    fn a_missing_file_gets_the_entry_alone() {
        let path = file();
        let at = place(&path, &["mcpServers"], JsonDialect::Strict);
        for text in [None, Some(""), Some("  \n")] {
            let added =
                edit_json(text, &at, &muster(), &entry(false), EntryChange::Add, false).unwrap();
            let written = added.text().clone().unwrap();
            assert_eq!(
                parsed(&written),
                json!({"mcpServers": {"muster": {"command": "/bin/muster", "args": ["mcp"]}}})
            );
            assert!(written.ends_with('\n'), "{written:?}");
        }
        let removed = edit_json(
            None,
            &at,
            &muster(),
            &entry(false),
            EntryChange::Remove,
            false,
        )
        .unwrap();
        assert_eq!(removed.outcome(), EditOutcome::Absent);
        assert!(removed.text().is_none());
    }

    /// A different entry under the name is kept unless forced, then
    /// replaced or removed.
    #[test]
    fn a_different_entry_needs_force() {
        let path = file();
        let at = place(&path, &["mcpServers"], JsonDialect::Strict);
        let theirs = r#"{"mcpServers": {"muster": {"command": "/old/muster", "args": ["mcp"]}}}"#;

        assert!(matches!(
            edit_json(
                Some(theirs),
                &at,
                &muster(),
                &entry(false),
                EntryChange::Add,
                false
            ),
            Err(AgentConfigError::Taken { .. })
        ));
        assert!(matches!(
            edit_json(
                Some(theirs),
                &at,
                &muster(),
                &entry(false),
                EntryChange::Remove,
                false
            ),
            Err(AgentConfigError::NotOurs { .. })
        ));
        let replaced = edit_json(
            Some(theirs),
            &at,
            &muster(),
            &entry(false),
            EntryChange::Add,
            true,
        )
        .unwrap();
        assert_eq!(replaced.outcome(), EditOutcome::Replaced);
        assert_eq!(
            parsed(replaced.text().as_ref().unwrap())["mcpServers"]["muster"]["command"],
            "/bin/muster"
        );
        let removed = edit_json(
            Some(theirs),
            &at,
            &muster(),
            &entry(false),
            EntryChange::Remove,
            true,
        )
        .unwrap();
        assert_eq!(
            parsed(removed.text().as_ref().unwrap()),
            json!({"mcpServers": {}})
        );
    }

    /// An entry Claude Code wrote itself, with `type` and `env`, is muster's
    /// own when it runs the same program.
    #[test]
    fn extra_keys_do_not_make_an_entry_different() {
        let path = file();
        let at = place(&path, &["mcpServers"], JsonDialect::Strict);
        let own = r#"{"mcpServers": {"muster": {"type": "stdio", "command": "/bin/muster", "args": ["mcp"], "env": {}}}}"#;

        let again = edit_json(
            Some(own),
            &at,
            &muster(),
            &entry(false),
            EntryChange::Add,
            false,
        )
        .unwrap();
        assert_eq!(again.outcome(), EditOutcome::AlreadyAdded);
    }

    /// A file that does not parse, or whose table is not an object, is
    /// refused and never rewritten; a strict file refuses comments.
    #[test]
    fn a_file_muster_cannot_read_is_refused() {
        let path = file();
        let strict = place(&path, &["mcpServers"], JsonDialect::Strict);
        let loose = place(&path, &["mcpServers"], JsonDialect::WithComments);
        for broken in ["{\"mcpServers\": {", "[1, 2]", "// note\n{}"] {
            assert!(
                matches!(
                    edit_json(
                        Some(broken),
                        &strict,
                        &muster(),
                        &entry(false),
                        EntryChange::Add,
                        false
                    ),
                    Err(AgentConfigError::Unparseable { .. } | AgentConfigError::NotATable { .. })
                ),
                "{broken}"
            );
        }
        assert!(matches!(
            edit_json(Some("{\"mcpServers\": []}"), &loose, &muster(), &entry(false), EntryChange::Add, false),
            Err(AgentConfigError::NotATable { key, .. }) if key == "mcpServers"
        ));
        assert!(matches!(
            edit_json(
                Some("{\"mcpServers\": 3}"),
                &loose,
                &muster(),
                &entry(false),
                EntryChange::Remove,
                false
            ),
            Err(AgentConfigError::NotATable { .. })
        ));
    }
}
