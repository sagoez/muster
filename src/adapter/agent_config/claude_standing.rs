//! What Claude Code's own lists say of a server, read only to warn: muster
//! never edits them, and a file it cannot read just leaves the warning out.

use std::{collections::HashMap, path::Path};

use serde::Deserialize;
use serde_json::Value;

use super::config_file::read_text;

/// Claude Code's settings file, in its config folder or a project's.
const SETTINGS: &str = "settings.json";
/// A project's own Claude Code settings file, kept out of version control.
const LOCAL_SETTINGS: &str = "settings.local.json";
/// A project's Claude Code folder.
const PROJECT_DIR: &str = ".claude";

/// How Claude Code stands toward one server of a project's `.mcp.json`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ClaudeStanding {
    /// Turned off for the project, or turned down when it asked.
    TurnedOff,
    /// Used without asking.
    Approved,
    /// Asked about before it is used, or not known otherwise.
    Asks,
}

/// The parts of `.claude.json` muster reads to warn: each project's list
/// of servers turned off.
#[derive(Deserialize)]
struct ClaudeJson {
    /// Each project's settings, by its key.
    projects: Option<HashMap<String, ProjectLists>>,
}

/// One project's list of servers turned off in `.claude.json`.
#[derive(Deserialize)]
struct ProjectLists {
    /// The servers turned off for the project.
    #[serde(rename = "disabledMcpServers")]
    disabled: Option<Value>,
}

/// The parts of a Claude Code settings file muster reads to warn.
#[derive(Deserialize)]
struct SettingsLists {
    /// Servers of `.mcp.json` turned down.
    #[serde(rename = "disabledMcpjsonServers")]
    rejected: Option<Value>,
    /// Servers of `.mcp.json` approved.
    #[serde(rename = "enabledMcpjsonServers")]
    approved: Option<Value>,
    /// Whether every server of `.mcp.json` is approved.
    #[serde(rename = "enableAllProjectMcpServers")]
    approve_all: Option<Value>,
}

/// How Claude Code stands toward `name` in the `.mcp.json` of the project at
/// `root`, keyed `key` in its config: off when its config at `claude_json`
/// turns it off for the project or a settings file turns it down, approved
/// when a settings file approves it, else asked about. The user's files
/// are left out without a home, and an unreadable file is skipped.
pub(super) fn project_standing(
    claude_json: Option<&Path>,
    claude_dir: Option<&Path>,
    root: &Path,
    key: Option<&str>,
    name: &str,
) -> ClaudeStanding {
    let turned_off = claude_json
        .zip(key)
        .and_then(|(path, key)| {
            let config: ClaudeJson = read_json(path)?;
            config.projects?.remove(key)?.disabled
        })
        .is_some_and(|list| names(&list, name, false));
    let project = root.join(PROJECT_DIR);
    let settings: Vec<SettingsLists> = claude_dir
        .map(|dir| dir.join(SETTINGS))
        .into_iter()
        .chain([project.join(SETTINGS), project.join(LOCAL_SETTINGS)])
        .filter_map(|path| read_json(&path))
        .collect();
    let named = |list: &Option<Value>| list.as_ref().is_some_and(|list| names(list, name, true));
    if turned_off || settings.iter().any(|lists| named(&lists.rejected)) {
        ClaudeStanding::TurnedOff
    } else if settings.iter().any(|lists| {
        named(&lists.approved) || lists.approve_all.as_ref().and_then(Value::as_bool) == Some(true)
    }) {
        ClaudeStanding::Approved
    } else {
        ClaudeStanding::Asks
    }
}

/// Whether `list` names `name`: a list of names, or, where Claude Code
/// takes one as `single` allows, a lone name.
pub(super) fn names(list: &Value, name: &str, single: bool) -> bool {
    match list {
        Value::Array(names) => names.iter().any(|listed| listed.as_str() == Some(name)),
        Value::String(listed) => single && listed == name,
        _ => false,
    }
}

/// The parts of the JSON file at `path` that `T` names, or none when it is
/// missing or cannot be read or parsed.
fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Option<T> {
    let text = read_text(path, path).ok()??;
    serde_json::from_str(text.text()).ok()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    /// The standing of `muster` in a scratch project whose files are given.
    fn standing(dir: &TempDir, files: &[(&str, Value)]) -> ClaudeStanding {
        let base = dir.path();
        for (path, value) in files {
            let path = base.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, value.to_string()).unwrap();
        }
        project_standing(
            Some(&base.join(".claude.json")),
            Some(&base.join(".claude")),
            &base.join("project"),
            Some("/p"),
            "muster",
        )
    }

    /// A server is off when its project's list in `.claude.json` or any
    /// settings file's turned-down list names it, approved when a settings
    /// file approves it or every server, and asked about otherwise.
    #[test]
    fn each_list_says_how_claude_code_stands() {
        for (files, expected) in [
            (vec![], ClaudeStanding::Asks),
            (
                vec![(
                    ".claude.json",
                    json!({"projects": {"/p": {"disabledMcpServers": ["muster"]}}}),
                )],
                ClaudeStanding::TurnedOff,
            ),
            (
                vec![(
                    ".claude.json",
                    json!({"projects": {"/other": {"disabledMcpServers": ["muster"]}}}),
                )],
                ClaudeStanding::Asks,
            ),
            (
                vec![(
                    "project/.claude/settings.local.json",
                    json!({"disabledMcpjsonServers": "muster"}),
                )],
                ClaudeStanding::TurnedOff,
            ),
            (
                vec![
                    (
                        ".claude/settings.json",
                        json!({"enabledMcpjsonServers": ["muster"]}),
                    ),
                    (
                        "project/.claude/settings.json",
                        json!({"disabledMcpjsonServers": ["muster"]}),
                    ),
                ],
                ClaudeStanding::TurnedOff,
            ),
            (
                vec![(
                    ".claude/settings.json",
                    json!({"enabledMcpjsonServers": ["muster"]}),
                )],
                ClaudeStanding::Approved,
            ),
            (
                vec![(
                    "project/.claude/settings.json",
                    json!({"enableAllProjectMcpServers": true}),
                )],
                ClaudeStanding::Approved,
            ),
        ] {
            let dir = TempDir::new().unwrap();
            let case = format!("{files:?}");
            assert_eq!(standing(&dir, &files), expected, "{case}");
        }
    }

    /// A file that does not parse, or a list of another shape, is skipped
    /// rather than failing.
    #[test]
    fn an_unreadable_file_is_skipped() {
        let dir = TempDir::new().unwrap();
        fs::create_dir_all(dir.path().join(".claude")).unwrap();
        fs::write(dir.path().join(".claude.json"), "{ broken").unwrap();
        fs::write(
            dir.path().join(".claude/settings.json"),
            r#"{"disabledMcpjsonServers": {"muster": true}}"#,
        )
        .unwrap();
        assert_eq!(standing(&dir, &[]), ClaudeStanding::Asks);
        assert!(!names(&json!("muster"), "muster", false));
        assert!(names(&json!(["x", "muster"]), "muster", false));
    }
}
