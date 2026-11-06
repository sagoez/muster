//! An edit of one server entry in a JSON or JSONC file, made on its concrete
//! syntax tree so every other key, comment and blank line stays as written.

use std::collections::HashSet;

use jsonc_parser::{
    ParseOptions,
    cst::{CstInputValue, CstNode, CstObject, CstObjectProp, CstRootNode},
};
use serde::de::IgnoredAny;

use super::{
    agent_config_error::AgentConfigError,
    disabling::Disabling,
    edit_outcome::EditOutcome,
    edited::Edited,
    entry_action::EntryAction,
    entry_change::EntryChange,
    entry_match::EntryMatch,
    json_dialect::JsonDialect,
    json_place::JsonPlace,
    server_entry::{COMMAND_KEY, ENABLED_KEY, ServerEntry},
    server_name::McpServerName,
};

/// The text a file muster creates starts from.
const EMPTY_OBJECT: &str = "{}\n";
/// The line break a file muster writes ends with.
const LINE_BREAK: char = '\n';

/// `change` applied to the entry `name` in `text`, the file at `place`, or
/// in an empty file when there is none, the entry turned off as
/// `disabling` says. A removal takes out the objects it leaves empty on the
/// way to the entry; turning the entry back on clears its own flag. The new
/// text is none when the file stays as it was.
///
/// # Errors
/// Returns [`AgentConfigError::Unparseable`],
/// [`AgentConfigError::NotAnObject`], [`AgentConfigError::NotATable`] or
/// [`AgentConfigError::DuplicateKey`] for a file muster cannot edit safely,
/// and [`AgentConfigError::Taken`] or [`AgentConfigError::NotOurs`] for
/// another server's entry under `name` without `force`.
pub(super) fn edit_json(
    text: Option<&str>,
    place: &JsonPlace<'_>,
    name: &McpServerName,
    entry: &ServerEntry,
    disabling: Disabling,
    change: EntryChange,
    force: bool,
) -> Result<Edited, AgentConfigError> {
    let (root, top) = parsed(text, place)?;
    let Some((table, holders)) = table_of(&top, place, change == EntryChange::Add)? else {
        return Ok(Edited::builder()
            .outcome(EditOutcome::Absent)
            .text(None)
            .build());
    };
    let existing = table.get(name.as_ref());
    if let Some(object) = existing.as_ref().and_then(CstObjectProp::object_value) {
        unique_keys(&object, place)?;
    }
    let judged = existing
        .as_ref()
        .map(|prop| judge(prop.value().as_ref(), entry));
    let action = EntryAction::decide(
        judged.map(|(matched, _)| matched),
        disabling == Disabling::Flag && judged.is_some_and(|(_, off)| off),
        change,
        force,
        place.path(),
        name,
    )?;
    let object = existing.as_ref().and_then(CstObjectProp::object_value);
    let point_at_this = |object: &CstObject| {
        if let Some(command) = object.get(COMMAND_KEY) {
            command.set_value(CstInputValue::from(entry.command().as_str()));
        }
    };
    match (action, existing) {
        (EntryAction::Insert, _) => {
            table.append(name.as_ref(), entry.json_input());
        },
        (EntryAction::Update, _) => object.iter().for_each(point_at_this),
        (EntryAction::Enable, _) => {
            object.iter().for_each(point_at_this);
            if let Some(flag) = object.as_ref().and_then(|object| object.get(ENABLED_KEY)) {
                flag.remove();
            }
        },
        (EntryAction::Replace, Some(prop)) => prop.set_value(entry.json_input()),
        (EntryAction::Remove, Some(prop)) => {
            prop.remove();
            prune(&holders);
        },
        _ => {},
    }
    let outcome = action.outcome();
    let text = outcome
        .writes()
        .then(|| finished(&root))
        .filter(|written| Some(written.trim_end()) != text.map(str::trim_end));
    Ok(Edited::builder().outcome(outcome).text(text).build())
}

/// `text`, the file at `place`, parsed, with its top object; an empty file,
/// or one of comments alone, is an empty object.
///
/// # Errors
/// Returns [`AgentConfigError::Unparseable`] for text that does not parse
/// in the place's dialect, [`AgentConfigError::NotAnObject`] for a file
/// holding something else, or [`AgentConfigError::DuplicateKey`] for a top
/// object holding a key twice.
fn parsed(
    text: Option<&str>,
    place: &JsonPlace<'_>,
) -> Result<(CstRootNode, CstObject), AgentConfigError> {
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
    let top = root
        .object_value_or_create()
        .ok_or_else(|| AgentConfigError::NotAnObject(place.path().to_path_buf()))?;
    unique_keys(&top, place)?;
    Ok((root, top))
}

/// The text of `root`, ending in a line break.
fn finished(root: &CstRootNode) -> String {
    let mut written = root.to_string();
    if !written.ends_with(LINE_BREAK) {
        written.push(LINE_BREAK);
    }
    written
}

/// The table of servers under `top`, with the property holding each object
/// on the way to it, outermost first; made with those objects when
/// `create`, or none when it is absent and not made.
///
/// # Errors
/// Returns [`AgentConfigError::NotATable`] when a key on the way holds
/// something other than an object, or [`AgentConfigError::DuplicateKey`]
/// when an object on the way holds a key twice.
fn table_of(
    top: &CstObject,
    place: &JsonPlace<'_>,
    create: bool,
) -> Result<Option<(CstObject, Vec<CstObjectProp>)>, AgentConfigError> {
    let mut object = top.clone();
    let mut holders = Vec::new();
    for key in place.table() {
        let prop = match object.get(key) {
            Some(prop) => prop,
            None if create => object.append(key, CstInputValue::Object(Vec::new())),
            None => return Ok(None),
        };
        object = prop
            .object_value()
            .ok_or_else(|| AgentConfigError::NotATable {
                path: place.path().to_path_buf(),
                key: key.clone(),
            })?;
        unique_keys(&object, place)?;
        holders.push(prop);
    }
    Ok(Some((object, holders)))
}

/// Refuses `object` when it holds a key twice: agents disagree on which
/// one counts, so no edit of it is safe.
///
/// # Errors
/// Returns [`AgentConfigError::DuplicateKey`] naming the key.
fn unique_keys(object: &CstObject, place: &JsonPlace<'_>) -> Result<(), AgentConfigError> {
    let mut seen = HashSet::new();
    for key in object
        .properties()
        .iter()
        .filter_map(CstObjectProp::decoded_name)
    {
        if !seen.insert(key.clone()) {
            return Err(AgentConfigError::DuplicateKey {
                path: place.path().to_path_buf(),
                key,
            });
        }
    }
    Ok(())
}

/// Takes out the objects `holders` hold that a removal left empty,
/// innermost first, stopping at the first one still holding a key.
fn prune(holders: &[CstObjectProp]) {
    for prop in holders.iter().rev() {
        if !prop
            .object_value()
            .is_some_and(|object| object.properties().is_empty())
        {
            return;
        }
        prop.clone().remove();
    }
}

/// What `value`, an existing entry, is to muster, and whether it is turned
/// off.
fn judge(value: Option<&CstNode>, entry: &ServerEntry) -> (EntryMatch, bool) {
    value
        .and_then(CstNode::to_serde_value)
        .map_or((EntryMatch::Foreign, false), |value| {
            entry.judge_json(&value)
        })
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use serde_json::{Value, json};

    use super::*;
    use crate::adapter::agent_config::testing::{MUSTER_BINARY, entry, muster};

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

    /// `change` made to muster's entry in `text` at `at`.
    fn edited(
        text: Option<&str>,
        at: &JsonPlace<'_>,
        change: EntryChange,
        force: bool,
    ) -> Result<Edited, AgentConfigError> {
        edit_json(
            text,
            at,
            &muster(),
            &entry(false),
            Disabling::Flag,
            change,
            force,
        )
    }

    /// `text` as plain JSON.
    fn parsed(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    /// `text` as JSON with comments.
    fn loosely_parsed(text: &str) -> Value {
        CstRootNode::parse(text, &ParseOptions::default())
            .unwrap()
            .value()
            .unwrap()
            .to_serde_value()
            .unwrap()
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
        let zed = |text: &str, change| {
            edit_json(
                Some(text),
                &at,
                &muster(),
                &entry(true),
                Disabling::Flag,
                change,
                false,
            )
            .unwrap()
        };
        let added = zed(ZED_SETTINGS, EntryChange::Add);
        let text = added.text().clone().unwrap();

        assert_eq!(added.outcome(), EditOutcome::Added);
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
        let value = loosely_parsed(&text);
        assert_eq!(
            value["context_servers"]["muster"],
            json!({"command": MUSTER_BINARY, "args": ["mcp"], "env": {}})
        );
        assert_eq!(value["theme"], "One Dark");

        let again = zed(&text, EntryChange::Add);
        assert_eq!(again.outcome(), EditOutcome::AlreadyAdded);
        assert!(again.text().is_none(), "an identical entry writes nothing");

        let removed = zed(&text, EntryChange::Remove);
        let after = removed.text().clone().unwrap();
        assert_eq!(removed.outcome(), EditOutcome::Removed);
        assert!(
            after.contains("/* keep this one */") && after.contains("other-mcp"),
            "{after}"
        );
        assert!(!after.contains(MUSTER_BINARY), "{after}");
    }

    /// A Zed settings file of comments alone is empty settings: the entry
    /// is added below the comments.
    #[test]
    fn a_settings_file_of_comments_alone_gains_the_entry() {
        let path = file();
        let at = place(&path, &["context_servers"], JsonDialect::WithComments);
        let comments = "// Zed settings\n// nothing yet\n";
        let added = edited(Some(comments), &at, EntryChange::Add, false).unwrap();
        let text = added.text().clone().unwrap();
        assert!(text.starts_with(comments), "{text}");
        assert_eq!(
            loosely_parsed(&text)["context_servers"]["muster"]["command"],
            MUSTER_BINARY
        );
        let absent = edited(Some(comments), &at, EntryChange::Remove, false).unwrap();
        assert_eq!(absent.outcome(), EditOutcome::Absent);
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
        let added = edited(Some(original), &user, EntryChange::Add, false).unwrap();
        let value = parsed(added.text().as_ref().unwrap());
        assert_eq!(
            value["mcpServers"]["muster"],
            json!({"command": MUSTER_BINARY, "args": ["mcp"]})
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
        let added = edited(Some(original), &local, EntryChange::Add, false).unwrap();
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

    /// A removal leaves no empty table of servers behind, nor a project's
    /// object it emptied, but keeps any object still holding a key.
    #[test]
    fn a_removal_takes_out_the_objects_it_empties() {
        let path = file();
        let project = place(&path, &["mcpServers"], JsonDialect::Strict);
        let added = edited(None, &project, EntryChange::Add, false).unwrap();
        let removed = edited(
            added.text().as_deref(),
            &project,
            EntryChange::Remove,
            false,
        )
        .unwrap();
        assert_eq!(parsed(removed.text().as_ref().unwrap()), json!({}));

        let local = place(
            &path,
            &["projects", "/work/shop", "mcpServers"],
            JsonDialect::Strict,
        );
        let original = r#"{"projects": {"/work/other": {"allowedTools": []}}}"#;
        let added = edited(Some(original), &local, EntryChange::Add, false).unwrap();
        let removed = edited(added.text().as_deref(), &local, EntryChange::Remove, false).unwrap();
        assert_eq!(parsed(removed.text().as_ref().unwrap()), parsed(original));

        let kept = r#"{"projects": {"/work/shop": {"allowedTools": [], "mcpServers": {"muster": {"command": "/bin/muster", "args": ["mcp"]}}}}}"#;
        let removed = edited(Some(kept), &local, EntryChange::Remove, false).unwrap();
        assert_eq!(
            parsed(removed.text().as_ref().unwrap()),
            json!({"projects": {"/work/shop": {"allowedTools": []}}})
        );
    }

    /// A missing or empty file becomes one holding just the entry.
    #[test]
    fn a_missing_file_gets_the_entry_alone() {
        let path = file();
        let at = place(&path, &["mcpServers"], JsonDialect::Strict);
        for text in [None, Some(""), Some("  \n")] {
            let added = edited(text, &at, EntryChange::Add, false).unwrap();
            let written = added.text().clone().unwrap();
            assert_eq!(
                parsed(&written),
                json!({"mcpServers": {"muster": {"command": MUSTER_BINARY, "args": ["mcp"]}}})
            );
            assert!(written.ends_with('\n'), "{written:?}");
        }
        let removed = edited(None, &at, EntryChange::Remove, false).unwrap();
        assert_eq!(removed.outcome(), EditOutcome::Absent);
        assert!(removed.text().is_none());
    }

    /// Another server under the name is kept unless forced, then replaced
    /// or removed.
    #[test]
    fn another_servers_entry_needs_force() {
        let path = file();
        let at = place(&path, &["mcpServers"], JsonDialect::Strict);
        let theirs = r#"{"mcpServers": {"muster": {"command": "/opt/other-tool", "args": ["mcp"]}, "x": {}}}"#;

        assert!(matches!(
            edited(Some(theirs), &at, EntryChange::Add, false),
            Err(AgentConfigError::Taken { .. })
        ));
        assert!(matches!(
            edited(Some(theirs), &at, EntryChange::Remove, false),
            Err(AgentConfigError::NotOurs { .. })
        ));
        let replaced = edited(Some(theirs), &at, EntryChange::Add, true).unwrap();
        assert_eq!(replaced.outcome(), EditOutcome::Replaced);
        assert_eq!(
            parsed(replaced.text().as_ref().unwrap())["mcpServers"]["muster"]["command"],
            MUSTER_BINARY
        );
        let removed = edited(Some(theirs), &at, EntryChange::Remove, true).unwrap();
        assert_eq!(
            parsed(removed.text().as_ref().unwrap()),
            json!({"mcpServers": {"x": {}}})
        );
    }

    /// muster's entry from an older install's path is pointed at this
    /// binary without force, its other keys kept, and removed as muster's.
    #[test]
    fn an_older_muster_entry_is_updated_in_place() {
        let path = file();
        let at = place(&path, &["mcpServers"], JsonDialect::Strict);
        let older = r#"{"mcpServers": {"muster": {"command": "/old/0.9.0/bin/muster", "args": ["mcp"], "env": {"A": "1"}}}}"#;

        let updated = edited(Some(older), &at, EntryChange::Add, false).unwrap();
        assert_eq!(updated.outcome(), EditOutcome::Updated);
        assert_eq!(
            parsed(updated.text().as_ref().unwrap())["mcpServers"]["muster"],
            json!({"command": MUSTER_BINARY, "args": ["mcp"], "env": {"A": "1"}})
        );
        let removed = edited(Some(older), &at, EntryChange::Remove, false).unwrap();
        assert_eq!(removed.outcome(), EditOutcome::Removed);
    }

    /// A turned-off entry of muster's is reported as such and left off
    /// without force, and turned back on in place with it: its flag
    /// cleared, its command pointed at this binary, every other key kept.
    #[test]
    fn a_turned_off_entry_is_reported_not_added() {
        let path = file();
        let at = place(&path, &["context_servers"], JsonDialect::WithComments);
        let off = r#"{"context_servers": {"muster": {"command": "/old/bin/muster", "args": ["mcp"], "env": {"A": "1"}, "enabled": false}}}"#;

        let kept = edited(Some(off), &at, EntryChange::Add, false).unwrap();
        assert_eq!(kept.outcome(), EditOutcome::Disabled);
        assert!(kept.text().is_none());
        let forced = edited(Some(off), &at, EntryChange::Add, true).unwrap();
        assert_eq!(forced.outcome(), EditOutcome::Enabled);
        assert_eq!(
            loosely_parsed(forced.text().as_ref().unwrap())["context_servers"]["muster"],
            json!({"command": MUSTER_BINARY, "args": ["mcp"], "env": {"A": "1"}})
        );
    }

    /// Where the agent turns a server off outside the entry, as Claude Code
    /// does, no key in the entry turns it off: muster's own reads as
    /// added, `enabled: false` or not, and force leaves it as it is.
    #[test]
    fn an_entry_turned_off_elsewhere_reads_as_added() {
        let path = file();
        let at = place(&path, &["mcpServers"], JsonDialect::Strict);
        for own in [
            r#"{"mcpServers": {"muster": {"command": "/bin/muster", "args": ["mcp"], "disabled": true}}}"#,
            r#"{"mcpServers": {"muster": {"command": "/bin/muster", "args": ["mcp"], "enabled": false}}}"#,
        ] {
            for force in [false, true] {
                let added = edit_json(
                    Some(own),
                    &at,
                    &muster(),
                    &entry(false),
                    Disabling::Elsewhere,
                    EntryChange::Add,
                    force,
                )
                .unwrap();
                assert_eq!(added.outcome(), EditOutcome::AlreadyAdded, "{own}");
                assert!(added.text().is_none(), "{own}");
            }
        }
    }

    /// An entry Claude Code wrote itself, with `type` and `env`, is muster's
    /// own when it runs the same program.
    #[test]
    fn extra_keys_do_not_make_an_entry_different() {
        let path = file();
        let at = place(&path, &["mcpServers"], JsonDialect::Strict);
        let own = r#"{"mcpServers": {"muster": {"type": "stdio", "command": "/bin/muster", "args": ["mcp"], "env": {}}}}"#;

        let again = edited(Some(own), &at, EntryChange::Add, false).unwrap();
        assert_eq!(again.outcome(), EditOutcome::AlreadyAdded);
    }

    /// A key repeated in the table, its entry or any object on the way to
    /// it is refused for add and remove alike, since agents read the last
    /// copy and an edit would act on another.
    #[test]
    fn a_repeated_key_on_the_way_is_refused() {
        let path = file();
        let local = place(
            &path,
            &["projects", "/work/shop", "mcpServers"],
            JsonDialect::Strict,
        );
        for (repeated, key) in [
            (
                r#"{"projects": {"/work/shop": {"mcpServers": {"muster": {"command": "/bin/muster", "args": ["mcp"]}, "muster": {"command": "/tmp/evil", "args": []}}}}}"#,
                "muster",
            ),
            (
                r#"{"projects": {"/work/shop": {"mcpServers": {}}, "/work/shop": {}}}"#,
                "/work/shop",
            ),
            (r#"{"projects": {}, "projects": {}}"#, "projects"),
            (
                r#"{"projects": {"/work/shop": {"mcpServers": {"muster": {"command": "/bin/muster", "command": "/tmp/evil", "args": ["mcp"]}}}}}"#,
                "command",
            ),
        ] {
            for change in [EntryChange::Add, EntryChange::Remove] {
                let refused = edited(Some(repeated), &local, change, true);
                assert!(
                    matches!(
                        &refused,
                        Err(AgentConfigError::DuplicateKey { key: found, .. }) if found == key
                    ),
                    "{repeated} {change}: {refused:?}"
                );
            }
        }
        let elsewhere = r#"{"other": {"a": 1, "a": 2}, "projects": {}}"#;
        assert!(edited(Some(elsewhere), &local, EntryChange::Add, false).is_ok());
    }

    /// A file that does not parse, or whose table is not an object, is
    /// refused and never rewritten; a strict file refuses comments.
    #[test]
    fn a_file_muster_cannot_read_is_refused() {
        let path = file();
        let strict = place(&path, &["mcpServers"], JsonDialect::Strict);
        let loose = place(&path, &["mcpServers"], JsonDialect::WithComments);
        for broken in ["{\"mcpServers\": {", "// note\n{}"] {
            assert!(
                matches!(
                    edited(Some(broken), &strict, EntryChange::Add, false),
                    Err(AgentConfigError::Unparseable { .. })
                ),
                "{broken}"
            );
        }
        assert!(matches!(
            edited(Some("[1, 2]"), &strict, EntryChange::Add, false),
            Err(AgentConfigError::NotAnObject(_))
        ));
        assert!(matches!(
            edited(Some("{\"mcpServers\": []}"), &loose, EntryChange::Add, false),
            Err(AgentConfigError::NotATable { key, .. }) if key == "mcpServers"
        ));
        assert!(matches!(
            edited(
                Some("{\"mcpServers\": 3}"),
                &loose,
                EntryChange::Remove,
                false
            ),
            Err(AgentConfigError::NotATable { .. })
        ));
    }
}
