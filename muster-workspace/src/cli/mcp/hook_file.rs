//! An agent's hooks file (Claude Code's `settings.json`, Codex's
//! `hooks.json`), into which `muster mcp add` puts a session-start hook
//! running `muster mcp context`, and from which `muster mcp remove` takes it.
//! Muster's hook is known by its command alone, so every other key and hook
//! in the file is kept as it was.

use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use getset::Getters;
use muster_domain::constants::{APP_NAME, MCP_SUBCOMMAND};
use serde_json::{Map, Value, json};
use typed_builder::TypedBuilder;

use crate::cli::{
    mcp::{hook_file_error::HookFileError, mcp_command::CONTEXT_SUBCOMMAND},
    transfer::text_file::write_atomically,
};

/// The key of the hooks object, by event.
const HOOKS_KEY: &str = "hooks";
/// The event whose hooks run as a session starts, resumes, is cleared or
/// is compacted.
const SESSION_START_KEY: &str = "SessionStart";
/// The key of a hook group's source filter.
const MATCHER_KEY: &str = "matcher";
/// The key of a handler's kind.
const TYPE_KEY: &str = "type";
/// The key of a command handler's command line.
const COMMAND_KEY: &str = "command";
/// The key of a handler's time limit, in seconds.
const TIMEOUT_KEY: &str = "timeout";
/// The kind of handler that runs a command line.
const COMMAND_HANDLER: &str = "command";
/// The session sources the hook runs on: a new session, a resumed one, one
/// cleared and one compacted, each of which starts from a fresh context.
pub const SESSION_SOURCES: &str = "^(startup|resume|clear|compact)$";
/// How long the agent lets the hook run, in seconds; it only reads the
/// database.
const HOOK_TIMEOUT_S: u64 = 10;
/// Between the words of a command shlex cannot quote.
const WORD_SEPARATOR: &str = " ";
/// What a shape refusal calls each part of the file, and what it should be.
const WHOLE_FILE: &str = "a top level";
const HOOKS_PART: &str = "a \"hooks\" entry";
const SESSION_START_PART: &str = "a \"hooks\".\"SessionStart\" entry";
const AN_OBJECT: &str = "an object";
const A_LIST: &str = "a list";
/// What the file holds once indented, before its final line break.
const LINE_BREAK: &str = "\n";

/// One agent's hooks file, which may not exist yet.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct HookFile {
    /// Where the agent reads it.
    path: PathBuf,
}

impl HookFile {
    /// Puts muster's session-start hook running `command` into the file,
    /// creating it and its folder when absent, and replacing a muster hook
    /// already there. Returns whether the file changed: running it twice
    /// writes once.
    ///
    /// # Errors
    /// Returns [`HookFileError`] when the file cannot be read or written, is
    /// not JSON, or holds its hooks in another shape; it is then untouched.
    pub fn install(&self, command: &str) -> Result<bool, HookFileError> {
        let mut settings = self.read()?.unwrap_or_else(|| Value::Object(Map::new()));
        if !with_hook(&mut settings, command, &self.path)? {
            return Ok(false);
        }
        self.write(&settings)?;
        Ok(true)
    }

    /// Takes muster's session-start hook out of the file, and the hook
    /// group and event entries it leaves empty, keeping everything else.
    /// Returns whether the file changed; an absent file is left absent.
    ///
    /// # Errors
    /// Returns [`HookFileError`] as [`Self::install`] does.
    pub fn uninstall(&self) -> Result<bool, HookFileError> {
        let Some(mut settings) = self.read()? else {
            return Ok(false);
        };
        if !without_hook(&mut settings, &self.path)? {
            return Ok(false);
        }
        self.write(&settings)?;
        Ok(true)
    }

    /// The file's JSON, an empty or blank file reading as an empty object;
    /// none when it does not exist.
    ///
    /// # Errors
    /// Returns [`HookFileError::Read`] or [`HookFileError::Malformed`].
    fn read(&self) -> Result<Option<Value>, HookFileError> {
        let text = match fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
            Err(source) => {
                return Err(HookFileError::Read {
                    path: self.path.clone(),
                    source,
                });
            },
        };
        if text.trim().is_empty() {
            return Ok(Some(Value::Object(Map::new())));
        }
        serde_json::from_str(&text)
            .map(Some)
            .map_err(|source| HookFileError::Malformed {
                path: self.path.clone(),
                source,
            })
    }

    /// Writes `settings` indented, atomically, through a symlink to the
    /// file it names so a linked dotfile stays linked.
    ///
    /// # Errors
    /// Returns [`HookFileError::Write`] when the folder or file cannot be
    /// written.
    fn write(&self, settings: &Value) -> Result<(), HookFileError> {
        let failed = |source| HookFileError::Write {
            path: self.path.clone(),
            source,
        };
        let target = fs::canonicalize(&self.path).unwrap_or_else(|_| self.path.clone());
        if let Some(folder) = target.parent() {
            fs::create_dir_all(folder).map_err(failed)?;
        }
        let mut text =
            serde_json::to_string_pretty(settings).map_err(|error| failed(error.into()))?;
        text.push_str(LINE_BREAK);
        write_atomically(&target, &text).map_err(failed)
    }
}

/// The hook's command line running `binary`'s `mcp context`, quoted for the
/// shell the agent runs it in.
pub fn context_command(binary: &Path) -> String {
    let binary = binary.to_string_lossy();
    let words = [binary.as_ref(), MCP_SUBCOMMAND, CONTEXT_SUBCOMMAND];
    shlex::try_join(words).unwrap_or_else(|_| words.join(WORD_SEPARATOR))
}

/// Whether `command` is muster's hook: a binary named `muster`, wherever it
/// lives, running `mcp context` and nothing else.
pub fn is_context_command(command: &str) -> bool {
    let Some(words) = shlex::split(command) else {
        return false;
    };
    match words.as_slice() {
        [binary, mcp, context] => {
            mcp == MCP_SUBCOMMAND
                && context == CONTEXT_SUBCOMMAND
                && Path::new(binary)
                    .file_name()
                    .is_some_and(|name| name == APP_NAME)
        },
        _ => false,
    }
}

/// The hook group muster adds: its sources and one handler running
/// `command`.
fn hook_group(command: &str) -> Value {
    json!({
        MATCHER_KEY: SESSION_SOURCES,
        HOOKS_KEY: [{
            TYPE_KEY: COMMAND_HANDLER,
            COMMAND_KEY: command,
            TIMEOUT_KEY: HOOK_TIMEOUT_S,
        }],
    })
}

/// Whether `handler` runs muster's hook.
fn is_muster_handler(handler: &Value) -> bool {
    handler
        .get(COMMAND_KEY)
        .and_then(Value::as_str)
        .is_some_and(is_context_command)
}

/// Puts muster's hook group running `command` last among `settings`'
/// session-start groups, after taking out any muster handler already there.
/// Returns whether `settings` changed: not when that group is already the
/// one muster handler.
///
/// # Errors
/// Returns [`HookFileError::Unexpected`] for a file at `path` whose top
/// level, hooks or session-start entry has another shape.
fn with_hook(settings: &mut Value, command: &str, path: &Path) -> Result<bool, HookFileError> {
    let wanted = hook_group(command);
    let root = as_object(settings, WHOLE_FILE, path)?;
    let hooks = as_object(
        root.entry(HOOKS_KEY)
            .or_insert_with(|| Value::Object(Map::new())),
        HOOKS_PART,
        path,
    )?;
    let groups = as_list(
        hooks
            .entry(SESSION_START_KEY)
            .or_insert_with(|| Value::Array(Vec::new())),
        path,
    )?;
    let mine: Vec<&Value> = groups
        .iter()
        .filter(|group| handlers(group).iter().any(is_muster_handler))
        .collect();
    if mine.as_slice() == [&wanted] {
        return Ok(false);
    }
    strip_muster_handlers(groups);
    groups.push(wanted);
    Ok(true)
}

/// Takes every muster handler out of `settings`' session-start groups, then
/// the groups, the event entry and the hooks object that leaves empty.
/// Returns whether `settings` changed.
///
/// # Errors
/// Returns [`HookFileError::Unexpected`] as [`with_hook`] does.
fn without_hook(settings: &mut Value, path: &Path) -> Result<bool, HookFileError> {
    let root = as_object(settings, WHOLE_FILE, path)?;
    let Some(hooks) = root.get_mut(HOOKS_KEY) else {
        return Ok(false);
    };
    let hooks = as_object(hooks, HOOKS_PART, path)?;
    let Some(groups) = hooks.get_mut(SESSION_START_KEY) else {
        return Ok(false);
    };
    let groups = as_list(groups, path)?;
    if !strip_muster_handlers(groups) {
        return Ok(false);
    }
    if groups.is_empty() {
        hooks.remove(SESSION_START_KEY);
    }
    if hooks.is_empty() {
        root.remove(HOOKS_KEY);
    }
    Ok(true)
}

/// Takes every muster handler out of `groups`, and each group that leaves
/// with no handler. Returns whether any was taken.
fn strip_muster_handlers(groups: &mut Vec<Value>) -> bool {
    let mut stripped = false;
    groups.retain_mut(|group| {
        let Some(list) = group.get_mut(HOOKS_KEY).and_then(Value::as_array_mut) else {
            return true;
        };
        let before = list.len();
        list.retain(|handler| !is_muster_handler(handler));
        if list.len() == before {
            return true;
        }
        stripped = true;
        !list.is_empty()
    });
    stripped
}

/// The handlers of `group`, none when it holds no list of them.
fn handlers(group: &Value) -> &[Value] {
    group
        .get(HOOKS_KEY)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// `value` as an object, or the refusal naming it as `what` in `path`.
///
/// # Errors
/// Returns [`HookFileError::Unexpected`] when it is something else.
fn as_object<'v>(
    value: &'v mut Value,
    what: &'static str,
    path: &Path,
) -> Result<&'v mut Map<String, Value>, HookFileError> {
    value
        .as_object_mut()
        .ok_or_else(|| HookFileError::Unexpected {
            path: path.to_path_buf(),
            what,
            expected: AN_OBJECT,
        })
}

/// `value`, the session-start entry, as a list, or the refusal naming it in
/// `path`.
///
/// # Errors
/// Returns [`HookFileError::Unexpected`] when it is something else.
fn as_list<'v>(value: &'v mut Value, path: &Path) -> Result<&'v mut Vec<Value>, HookFileError> {
    value
        .as_array_mut()
        .ok_or_else(|| HookFileError::Unexpected {
            path: path.to_path_buf(),
            what: SESSION_START_PART,
            expected: A_LIST,
        })
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::symlink;

    use tempfile::TempDir;

    use super::*;

    /// The command muster's hook runs in these tests.
    const COMMAND: &str = "/opt/bin/muster mcp context";
    /// A user's own session-start hooks and an unrelated setting.
    const OTHERS: &str = r#"{
  "model": "opus",
  "hooks": {
    "SessionStart": [
      {"hooks": [{"type": "command", "command": "python3 login-check.py", "timeout": 10}]},
      {"matcher": "^(startup|resume)$", "hooks": [{"type": "command", "command": "bash herdr.sh session"}]}
    ],
    "Stop": [{"hooks": [{"type": "command", "command": "notify-send done"}]}]
  }
}"#;

    /// A hooks file at `settings.json` in a fresh scratch folder.
    fn scratch_file() -> (HookFile, TempDir) {
        let scratch = TempDir::new().unwrap();
        let file = HookFile::builder()
            .path(scratch.path().join(".claude").join("settings.json"))
            .build();
        (file, scratch)
    }

    /// The JSON the file holds now.
    fn json_of(file: &HookFile) -> Value {
        serde_json::from_str(&fs::read_to_string(file.path()).unwrap()).unwrap()
    }

    /// An absent file and its folder are created holding only the hook.
    #[test]
    fn an_absent_file_gets_only_the_hook() {
        let (file, _scratch) = scratch_file();

        assert!(file.install(COMMAND).unwrap());

        assert_eq!(
            json_of(&file),
            json!({"hooks": {"SessionStart": [{
                "matcher": SESSION_SOURCES,
                "hooks": [{"type": "command", "command": COMMAND, "timeout": HOOK_TIMEOUT_S}],
            }]}})
        );
    }

    /// An empty file reads as an empty object.
    #[test]
    fn an_empty_file_gets_the_hook() {
        let (file, _scratch) = scratch_file();
        fs::create_dir_all(file.path().parent().unwrap()).unwrap();
        fs::write(file.path(), "").unwrap();

        assert!(file.install(COMMAND).unwrap());

        assert_eq!(
            json_of(&file)["hooks"]["SessionStart"][0],
            hook_group(COMMAND)
        );
    }

    /// Other hooks and keys stay as they were, the hook goes after them, and
    /// removing it gives back exactly the file it was added to.
    #[test]
    fn other_hooks_stay_and_remove_restores_them() {
        let (file, _scratch) = scratch_file();
        fs::create_dir_all(file.path().parent().unwrap()).unwrap();
        fs::write(file.path(), OTHERS).unwrap();
        let before: Value = serde_json::from_str(OTHERS).unwrap();

        assert!(file.install(COMMAND).unwrap());
        let added = json_of(&file);
        let groups = added["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(groups.len(), 3);
        assert_eq!(
            groups[..2],
            before["hooks"]["SessionStart"].as_array().unwrap()[..]
        );
        assert_eq!(groups[2], hook_group(COMMAND));
        assert_eq!(added["hooks"]["Stop"], before["hooks"]["Stop"]);
        assert_eq!(added["model"], before["model"]);

        assert!(file.uninstall().unwrap());
        assert_eq!(json_of(&file), before);
        assert!(!file.uninstall().unwrap(), "nothing left to remove");
    }

    /// Adding again changes nothing, and adding from a moved binary replaces
    /// the hook rather than adding a second.
    #[test]
    fn adding_again_never_duplicates_the_hook() {
        let (file, _scratch) = scratch_file();
        assert!(file.install(COMMAND).unwrap());
        let written = fs::read_to_string(file.path()).unwrap();

        assert!(!file.install(COMMAND).unwrap());
        assert_eq!(fs::read_to_string(file.path()).unwrap(), written);

        let moved = "'/home/me/my tools/muster' mcp context";
        assert!(file.install(moved).unwrap());
        let groups = json_of(&file)["hooks"]["SessionStart"].clone();
        assert_eq!(groups, json!([hook_group(moved)]));
    }

    /// A muster handler sharing a group with the user's own is taken out of
    /// it alone, the group staying with the user's handler.
    #[test]
    fn a_shared_group_keeps_the_users_handler() {
        let (file, _scratch) = scratch_file();
        fs::create_dir_all(file.path().parent().unwrap()).unwrap();
        let shared = json!({"hooks": {"SessionStart": [{"hooks": [
            {"type": "command", "command": "echo hi"},
            {"type": "command", "command": COMMAND},
        ]}]}});
        fs::write(file.path(), shared.to_string()).unwrap();

        assert!(file.uninstall().unwrap());

        assert_eq!(
            json_of(&file),
            json!({"hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": "echo hi"}]}]}})
        );
    }

    /// A file that is not JSON, or whose hooks are another shape, is
    /// refused and left byte for byte as it was.
    #[test]
    fn a_malformed_file_is_refused_unwritten() {
        let (file, _scratch) = scratch_file();
        fs::create_dir_all(file.path().parent().unwrap()).unwrap();
        for text in [
            "{\"hooks\": ",
            "[1, 2]",
            r#"{"hooks": []}"#,
            r#"{"hooks": {"SessionStart": {}}}"#,
        ] {
            fs::write(file.path(), text).unwrap();

            let added = file.install(COMMAND);
            let removed = file.uninstall();

            assert!(
                matches!(
                    added,
                    Err(HookFileError::Malformed { .. } | HookFileError::Unexpected { .. })
                ),
                "{text}: {added:?}"
            );
            assert!(removed.is_err(), "{text}: {removed:?}");
            assert_eq!(fs::read_to_string(file.path()).unwrap(), text);
        }
    }

    /// A symlinked file is written through, keeping the link.
    #[test]
    fn a_linked_file_stays_linked() {
        let (file, scratch) = scratch_file();
        let real = scratch.path().join("dotfiles.json");
        fs::write(&real, "{}").unwrap();
        fs::create_dir_all(file.path().parent().unwrap()).unwrap();
        symlink(&real, file.path()).unwrap();

        assert!(file.install(COMMAND).unwrap());

        assert!(
            fs::symlink_metadata(file.path())
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(
            json_of(&file)["hooks"]["SessionStart"][0],
            hook_group(COMMAND)
        );
    }

    /// Muster's hook is known by its command wherever the binary lives, and
    /// nothing else is taken for it.
    #[test]
    fn the_hook_is_known_by_its_command() {
        assert!(is_context_command(COMMAND));
        assert!(is_context_command("muster mcp context"));
        assert!(is_context_command(&context_command(Path::new(
            "/home/me/my tools/muster"
        ))));
        assert!(!is_context_command("muster mcp context --extra"));
        assert!(!is_context_command("/opt/bin/not-muster mcp context"));
        assert!(!is_context_command("echo muster mcp context"));
        assert!(!is_context_command(
            "bash '/home/me/.claude/hooks/herdr.sh' session"
        ));
    }
}
