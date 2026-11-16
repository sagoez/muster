//! An agent's hooks file (Claude Code's `settings.json`, Codex's
//! `hooks.json`), into which `muster mcp add` puts a session-start hook
//! running `muster mcp context`, and from which `muster mcp remove` takes it.
//! The file is edited as text through its syntax tree, so every key, its
//! order, the indentation and any comment stay byte for byte as they were.

use std::path::{Path, PathBuf};

use getset::Getters;
use jsonc_parser::{
    ParseOptions,
    cst::{CstArray, CstInputValue, CstNode, CstObject, CstRootNode},
};
use serde_json::Value;
use typed_builder::TypedBuilder;

use crate::cli::mcp::{
    agent_file::{link_target, read_optional, writable, write_through},
    agent_keys::{
        COMMAND_HANDLER, COMMAND_KEY, HOOKS_KEY, MATCHER_KEY, SESSION_START_KEY, TIMEOUT_KEY,
        TYPE_KEY,
    },
    hook_command::is_context_command,
    hook_file_error::HookFileError,
    hook_file_part::HookFilePart,
    json_shape::JsonShape,
};

/// The session sources the hook runs on: a new session, a resumed one, one
/// cleared and one compacted, each of which starts from a fresh context.
pub const SESSION_SOURCES: &str = "^(startup|resume|clear|compact)$";
/// How long the agent lets the hook run, in seconds; it only reads the
/// database, and gives up itself well before this.
pub const HOOK_TIMEOUT_S: u64 = 10;
/// How many times an edit starts over when the file changed under it.
const EDIT_ATTEMPTS: usize = 2;
/// The byte-order mark some editors put before a file's text, which the
/// parser refuses and the file keeps.
const BYTE_ORDER_MARK: char = '\u{feff}';

/// An edit of a hooks file's text: its new text, none when it needs no
/// change, or the refusal of it.
type Edited = Result<Option<String>, HookFileError>;

/// One agent's hooks file, which may not exist yet.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct HookFile {
    /// Where the agent reads it.
    path: PathBuf,
}

impl HookFile {
    /// Whether [`Self::install`] of `command` would change the file, without
    /// changing it.
    ///
    /// # Errors
    /// Returns what [`Self::install`] would, but a failed write.
    pub fn check_install(&self, command: &str) -> Result<bool, HookFileError> {
        self.check(|text, path| installed(text, command, path))
    }

    /// Whether [`Self::uninstall`] would change the file, without changing
    /// it.
    ///
    /// # Errors
    /// Returns what [`Self::uninstall`] would, but a failed write.
    pub fn check_uninstall(&self) -> Result<bool, HookFileError> {
        self.check(uninstalled)
    }

    /// Puts muster's session-start hook running `command` last among the
    /// file's session-start groups, creating the file and its folder when
    /// absent and replacing a muster hook already there. Returns whether the
    /// file changed: running it twice writes once.
    ///
    /// # Errors
    /// Returns [`HookFileError`] when the file cannot be read or written, is
    /// not JSON, holds its hooks in another shape or twice, or kept changing
    /// while muster edited it; it is then untouched.
    pub fn install(&self, command: &str) -> Result<bool, HookFileError> {
        self.apply(|text, path| installed(text, command, path))
    }

    /// Takes muster's session-start hook out of the file, and the group,
    /// event and hooks entries that leaves empty. The file itself stays,
    /// even holding only `{}`, as muster cannot know it made it. Returns
    /// whether the file changed; an absent file stays absent.
    ///
    /// # Errors
    /// Returns [`HookFileError`] as [`Self::install`] does.
    pub fn uninstall(&self) -> Result<bool, HookFileError> {
        self.apply(uninstalled)
    }

    /// Whether `edit` would change the file, refusing what it refuses and a
    /// file this user could not write.
    ///
    /// # Errors
    /// Returns the [`HookFileError`] of `edit`, of a read, or
    /// [`HookFileError::ReadOnly`].
    fn check(&self, edit: impl Fn(Option<&str>, &Path) -> Edited) -> Result<bool, HookFileError> {
        let text = read_optional(&self.path)?;
        if edit(text.as_deref(), &self.path)?.is_none() {
            return Ok(false);
        }
        writable(&link_target(&self.path)?)?;
        Ok(true)
    }

    /// Reads the file, edits it with `edit` and writes the result. When the
    /// file changed between the read and the write it starts over, which
    /// narrows the window in which another writer's change is lost but
    /// cannot close it. Returns whether it wrote.
    ///
    /// # Errors
    /// Returns the [`HookFileError`] of `edit`, of a read or a write,
    /// [`HookFileError::ReadOnly`], or [`HookFileError::Changed`] when the
    /// file changed on every attempt.
    fn apply(&self, edit: impl Fn(Option<&str>, &Path) -> Edited) -> Result<bool, HookFileError> {
        for _ in 0..EDIT_ATTEMPTS {
            let before = read_optional(&self.path)?;
            let Some(edited) = edit(before.as_deref(), &self.path)? else {
                return Ok(false);
            };
            writable(&link_target(&self.path)?)?;
            if read_optional(&self.path)? != before {
                continue;
            }
            write_through(&self.path, &edited)?;
            return Ok(true);
        }
        Err(HookFileError::Changed {
            path: self.path.clone(),
        })
    }
}

/// `text`, the file at `path`, as a syntax tree, and whether it opened with
/// a byte-order mark, which the tree leaves out. A file holding only
/// whitespace reads as an empty one, so a new object gets a clean indent.
///
/// # Errors
/// Returns [`HookFileError::Malformed`] when it is not JSON.
fn parsed(text: &str, path: &Path) -> Result<(CstRootNode, bool), HookFileError> {
    let unmarked = text.strip_prefix(BYTE_ORDER_MARK);
    let body = unmarked.unwrap_or(text);
    let body = if body.trim().is_empty() { "" } else { body };
    CstRootNode::parse(body, &ParseOptions::default())
        .map(|root| (root, unmarked.is_some()))
        .map_err(|error| HookFileError::Malformed {
            path: path.to_path_buf(),
            message: error.to_string(),
        })
}

/// `root`'s text, after the byte-order mark the file opened with.
fn written(root: &CstRootNode, marked: bool) -> String {
    let text = root.to_string();
    if marked {
        format!("{BYTE_ORDER_MARK}{text}")
    } else {
        text
    }
}

/// The refusal of `part` of the file at `path` for not being `shape`.
fn unexpected(path: &Path, part: HookFilePart, shape: JsonShape) -> HookFileError {
    HookFileError::Unexpected {
        path: path.to_path_buf(),
        part,
        shape,
    }
}

/// Refuses an `object` holding `key` more than once: the agents read the
/// last, the syntax tree edits the first, so muster could not tell what
/// runs.
///
/// # Errors
/// Returns [`HookFileError::Duplicate`] naming `part`.
fn once(
    object: &CstObject,
    key: &str,
    part: HookFilePart,
    path: &Path,
) -> Result<(), HookFileError> {
    let count = object
        .properties()
        .iter()
        .filter(|property| property.decoded_name().is_some_and(|name| name == key))
        .count();
    if count > 1 {
        return Err(HookFileError::Duplicate {
            path: path.to_path_buf(),
            part,
        });
    }
    Ok(())
}

/// The top-level object of `root`, the file at `path`; none for a file
/// holding no value.
///
/// # Errors
/// Returns [`HookFileError::Unexpected`] when the value is not an object.
fn top_object(root: &CstRootNode, path: &Path) -> Result<Option<CstObject>, HookFileError> {
    root.value()
        .map(|value| {
            value
                .as_object()
                .ok_or_else(|| unexpected(path, HookFilePart::TopLevel, JsonShape::Object))
        })
        .transpose()
}

/// The hooks object of `root`, the file at `path`, and its session-start
/// groups; none when it has none.
///
/// # Errors
/// Returns [`HookFileError::Unexpected`] when its top level, hooks or
/// session-start entry has another shape, or [`HookFileError::Duplicate`]
/// when one of those keys is there twice.
fn session_groups(
    root: &CstRootNode,
    path: &Path,
) -> Result<Option<(CstObject, CstArray)>, HookFileError> {
    let Some(object) = top_object(root, path)? else {
        return Ok(None);
    };
    once(&object, HOOKS_KEY, HookFilePart::Hooks, path)?;
    let Some(hooks) = object.get(HOOKS_KEY) else {
        return Ok(None);
    };
    let hooks = hooks
        .object_value()
        .ok_or_else(|| unexpected(path, HookFilePart::Hooks, JsonShape::Object))?;
    once(&hooks, SESSION_START_KEY, HookFilePart::SessionStart, path)?;
    let Some(groups) = hooks.get(SESSION_START_KEY) else {
        return Ok(None);
    };
    let groups = groups
        .array_value()
        .ok_or_else(|| unexpected(path, HookFilePart::SessionStart, JsonShape::List))?;
    Ok(Some((hooks, groups)))
}

/// The hook group muster adds, the one place its shape is written: its
/// sources and one handler running `command`, keys in the order the file
/// shows them.
fn hook_group_input(command: &str) -> CstInputValue {
    CstInputValue::Object(vec![
        (MATCHER_KEY.to_owned(), SESSION_SOURCES.into()),
        (
            HOOKS_KEY.to_owned(),
            CstInputValue::Array(vec![CstInputValue::Object(vec![
                (TYPE_KEY.to_owned(), COMMAND_HANDLER.into()),
                (COMMAND_KEY.to_owned(), command.into()),
                (TIMEOUT_KEY.to_owned(), HOOK_TIMEOUT_S.into()),
            ])]),
        ),
    ])
}

/// `input` as the value it writes.
fn tree_value(input: &CstInputValue) -> Value {
    match input {
        CstInputValue::Null => Value::Null,
        CstInputValue::Bool(flag) => Value::Bool(*flag),
        CstInputValue::Number(number) => serde_json::from_str(number).unwrap_or(Value::Null),
        CstInputValue::String(text) => Value::String(text.clone()),
        CstInputValue::Array(items) => Value::Array(items.iter().map(tree_value).collect()),
        CstInputValue::Object(fields) => Value::Object(
            fields
                .iter()
                .map(|(key, field)| (key.clone(), tree_value(field)))
                .collect(),
        ),
    }
}

/// Whether `handler` runs muster's hook.
fn is_muster_handler(handler: &CstNode) -> bool {
    handler.to_serde_value().is_some_and(|value| {
        value
            .get(COMMAND_KEY)
            .and_then(Value::as_str)
            .is_some_and(is_context_command)
    })
}

/// The handlers of `group`, none when it holds no list of them.
fn handlers(group: &CstNode) -> Vec<CstNode> {
    group
        .as_object()
        .and_then(|group| group.array_value(HOOKS_KEY))
        .map(|list| list.elements())
        .unwrap_or_default()
}

/// Whether `group` holds a muster handler.
fn holds_muster_handler(group: &CstNode) -> bool {
    handlers(group).iter().any(is_muster_handler)
}

/// Takes every muster handler out of `groups`, and each group that leaves
/// with no handler. Returns whether any was taken.
fn strip_muster_handlers(groups: &CstArray) -> bool {
    let mut stripped = false;
    for group in groups.elements() {
        let listed = handlers(&group);
        let mine: Vec<CstNode> = listed
            .iter()
            .filter(|handler| is_muster_handler(handler))
            .cloned()
            .collect();
        if mine.is_empty() {
            continue;
        }
        stripped = true;
        if mine.len() == listed.len() {
            group.remove();
        } else {
            mine.into_iter().for_each(CstNode::remove);
        }
    }
    stripped
}

/// `text`, the file at `path` or none when absent, with muster's hook
/// running `command` put in; none when it is already the one muster hook.
///
/// # Errors
/// Returns [`HookFileError::Malformed`], [`HookFileError::Unexpected`] or
/// [`HookFileError::Duplicate`].
fn installed(text: Option<&str>, command: &str, path: &Path) -> Edited {
    let (root, marked) = parsed(text.unwrap_or_default(), path)?;
    session_groups(&root, path)?;
    let object = match top_object(&root, path)? {
        Some(object) => object,
        None => root.object_value_or_set(),
    };
    let hooks = object.object_value_or_set(HOOKS_KEY);
    let groups = hooks.array_value_or_set(SESSION_START_KEY);
    let mine: Vec<CstNode> = groups
        .elements()
        .into_iter()
        .filter(holds_muster_handler)
        .collect();
    let wanted = hook_group_input(command);
    if let [only] = mine.as_slice()
        && only.to_serde_value() == Some(tree_value(&wanted))
    {
        return Ok(None);
    }
    strip_muster_handlers(&groups);
    groups.append(wanted);
    Ok(Some(written(&root, marked)))
}

/// `text`, the file at `path`, with every muster handler taken out, and the
/// group, event and hooks entries that leaves empty; none when it holds
/// none.
///
/// # Errors
/// Returns [`HookFileError::Malformed`], [`HookFileError::Unexpected`] or
/// [`HookFileError::Duplicate`].
fn uninstalled(text: Option<&str>, path: &Path) -> Edited {
    let Some(text) = text else {
        return Ok(None);
    };
    let (root, marked) = parsed(text, path)?;
    let Some((hooks, groups)) = session_groups(&root, path)? else {
        return Ok(None);
    };
    if !strip_muster_handlers(&groups) {
        return Ok(None);
    }
    if groups.elements().is_empty()
        && let Some(event) = hooks.get(SESSION_START_KEY)
    {
        event.remove();
    }
    if hooks.properties().is_empty()
        && let Some(object) = root.object_value()
        && let Some(entry) = object.get(HOOKS_KEY)
    {
        entry.remove();
    }
    Ok(Some(written(&root, marked)))
}

#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        fs::{self, Permissions},
        os::unix::fs::{PermissionsExt, symlink},
    };

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    /// The command muster's hook runs in these tests.
    const COMMAND: &str = "/opt/bin/muster mcp context --hook";
    /// A file someone keeps by hand: their own key order, a 4-space indent,
    /// comments, a trailing comma and their own session-start hooks.
    const HAND_KEPT: &str = r#"{
    // my settings, by hand
    "zeta": 1,
    "hooks": {
        "SessionStart": [
            {"hooks": [{"type": "command", "command": "python3 login-check.py"}]}, // login
            {
                "matcher": "^(startup|resume)$",
                "hooks": [{"type": "command", "command": "bash herdr.sh session"}],
            }
        ],
        "Stop": [{"hooks": [{"type": "command", "command": "notify-send done"}]}]
    },
    "alpha": true
}
"#;
    /// The same kind of file with no hooks at all.
    const NO_HOOKS: &str = "{\n    \"zeta\": 1,\n    /* kept */\n    \"alpha\": [1, 2]\n}\n";
    /// A file the user made holding only an empty object.
    const EMPTY_OBJECT: &str = "{}\n";
    /// How deep a hook handler's keys sit in those files: six levels of
    /// their 4-space indent.
    const HANDLER_INDENT: usize = 24;
    /// The mode of a file nobody may write.
    const READ_ONLY_FILE: u32 = 0o444;
    /// The mode of a folder its owner may enter and read but not write.
    const READ_ONLY_FOLDER: u32 = 0o555;
    /// The mode that lets the owner clean a folder up again.
    const OWNER_FOLDER: u32 = 0o755;

    /// muster's hook group running `command`, as a value.
    fn hook_group(command: &str) -> Value {
        tree_value(&hook_group_input(command))
    }

    /// A hooks file at `.claude/settings.json` in a fresh scratch folder.
    fn scratch_file() -> (HookFile, TempDir) {
        let scratch = TempDir::new().unwrap();
        let file = HookFile::builder()
            .path(scratch.path().join(".claude").join("settings.json"))
            .build();
        (file, scratch)
    }

    /// `file` holding `text`, its folder made.
    fn holding(file: &HookFile, text: &str) {
        fs::create_dir_all(file.path().parent().unwrap()).unwrap();
        fs::write(file.path(), text).unwrap();
    }

    /// The JSON the file holds now, comments allowed.
    fn json_of(file: &HookFile) -> Value {
        let text = fs::read_to_string(file.path()).unwrap();
        CstRootNode::parse(
            text.trim_start_matches(BYTE_ORDER_MARK),
            &ParseOptions::default(),
        )
        .unwrap()
        .to_serde_value()
        .unwrap()
    }

    /// An absent file and its folder are made holding only the hook; taking
    /// it out leaves the file, as an empty object.
    #[test]
    fn an_absent_file_gets_only_the_hook() {
        let (file, _scratch) = scratch_file();

        assert!(file.check_install(COMMAND).unwrap());
        assert!(!file.path().exists(), "a check writes nothing");
        assert!(file.install(COMMAND).unwrap());
        assert_eq!(
            json_of(&file),
            json!({"hooks": {"SessionStart": [hook_group(COMMAND)]}})
        );

        assert!(file.uninstall().unwrap());
        assert_eq!(json_of(&file), json!({}));
    }

    /// An empty file, or one of only whitespace, gets the hook with the
    /// same clean indentation an absent one does.
    #[test]
    fn an_empty_or_blank_file_gets_a_clean_indent() {
        let (absent, _scratch) = scratch_file();
        absent.install(COMMAND).unwrap();
        let clean = fs::read_to_string(absent.path()).unwrap();

        for text in ["", "  \n", "\n\n"] {
            let (file, _scratch) = scratch_file();
            holding(&file, text);

            assert!(file.install(COMMAND).unwrap());

            assert_eq!(fs::read_to_string(file.path()).unwrap(), clean, "{text:?}");
        }
    }

    /// Adding then removing on a file kept by hand gives back its exact
    /// bytes, its order, indentation, comments and byte-order mark
    /// included, and a file the user made as `{}` is kept, never deleted.
    #[test]
    fn add_then_remove_gives_back_the_same_bytes() {
        let marked = format!("{BYTE_ORDER_MARK}{NO_HOOKS}");
        for original in [HAND_KEPT, NO_HOOKS, marked.as_str(), EMPTY_OBJECT] {
            let (file, _scratch) = scratch_file();
            holding(&file, original);

            assert!(file.install(COMMAND).unwrap());
            let added = fs::read_to_string(file.path()).unwrap();
            if original != EMPTY_OBJECT {
                let indent = " ".repeat(HANDLER_INDENT);
                assert!(
                    added.contains(&format!("\n{indent}\"command\": \"{COMMAND}\",\n")),
                    "the file's own indent: {added}"
                );
            }
            assert_eq!(
                added.starts_with(BYTE_ORDER_MARK),
                original.starts_with(BYTE_ORDER_MARK)
            );
            assert!(file.uninstall().unwrap());

            assert_eq!(fs::read_to_string(file.path()).unwrap(), original);
        }
    }

    /// Adding again writes nothing, and adding from a binary moved and named
    /// otherwise replaces the hook rather than adding a second.
    #[test]
    fn adding_again_never_duplicates_the_hook() {
        let (file, _scratch) = scratch_file();
        holding(&file, HAND_KEPT);
        assert!(file.install(COMMAND).unwrap());
        let written = fs::read_to_string(file.path()).unwrap();

        assert!(!file.check_install(COMMAND).unwrap());
        assert!(!file.install(COMMAND).unwrap());
        assert_eq!(fs::read_to_string(file.path()).unwrap(), written);

        let moved = "'/home/me/my tools/muster-nightly' mcp context --hook";
        assert!(file.install(moved).unwrap());
        let groups = json_of(&file)["hooks"]["SessionStart"].clone();
        assert_eq!(groups.as_array().unwrap().len(), 3);
        assert_eq!(groups[2], hook_group(moved));
    }

    /// A muster handler sharing a group with the user's own is taken out of
    /// it alone, the group staying with the user's handler.
    #[test]
    fn a_shared_group_keeps_the_users_handler() {
        let (file, _scratch) = scratch_file();
        let shared = json!({"hooks": {"SessionStart": [{"hooks": [
            {"type": "command", "command": "echo hi"},
            {"type": "command", "command": COMMAND},
        ]}]}});
        holding(&file, &shared.to_string());

        assert!(file.uninstall().unwrap());

        assert_eq!(
            json_of(&file),
            json!({"hooks": {"SessionStart": [{"hooks": [{"type": "command", "command": "echo hi"}]}]}})
        );
    }

    /// A hook running a relative `./muster`, which a repository could ship,
    /// is not muster's: it is neither replaced nor removed.
    #[test]
    fn a_relative_muster_is_someone_elses_hook() {
        let (file, _scratch) = scratch_file();
        let planted = json!({"hooks": {"SessionStart": [{"hooks": [
            {"type": "command", "command": "./muster mcp context"},
        ]}]}})
        .to_string();
        holding(&file, &planted);

        assert!(!file.check_uninstall().unwrap());
        assert!(file.install(COMMAND).unwrap());
        let groups = json_of(&file)["hooks"]["SessionStart"].clone();
        assert_eq!(groups.as_array().unwrap().len(), 2, "{groups}");
    }

    /// A file that is not JSON, whose hooks are another shape, or that holds
    /// `hooks` or `SessionStart` twice is refused by the check and the edit
    /// alike and left byte for byte.
    #[test]
    fn a_malformed_file_is_refused_unwritten() {
        let (file, _scratch) = scratch_file();
        for text in [
            "{\"hooks\": ",
            "[1, 2]",
            r#"{"hooks": []}"#,
            r#"{"hooks": {"SessionStart": {}}}"#,
            r#"{"hooks": {"SessionStart": []}, "hooks": {"Stop": []}}"#,
            r#"{"hooks": {"SessionStart": [], "SessionStart": []}}"#,
        ] {
            holding(&file, text);

            assert!(file.check_install(COMMAND).is_err(), "{text}");
            assert!(file.install(COMMAND).is_err(), "{text}");
            assert!(file.check_uninstall().is_err(), "{text}");
            assert!(file.uninstall().is_err(), "{text}");
            assert_eq!(fs::read_to_string(file.path()).unwrap(), text);
        }
    }

    /// A read-only file, or one in a folder this user cannot write, is
    /// refused by the check too, and left as it was.
    #[test]
    fn a_file_this_user_cannot_write_is_refused() {
        let (file, _scratch) = scratch_file();
        holding(&file, NO_HOOKS);
        fs::set_permissions(file.path(), Permissions::from_mode(READ_ONLY_FILE)).unwrap();

        assert!(matches!(
            file.check_install(COMMAND),
            Err(HookFileError::ReadOnly { .. })
        ));
        assert!(matches!(
            file.install(COMMAND),
            Err(HookFileError::ReadOnly { .. })
        ));
        assert_eq!(fs::read_to_string(file.path()).unwrap(), NO_HOOKS);

        let (inside, _other) = scratch_file();
        holding(&inside, NO_HOOKS);
        let folder = inside.path().parent().unwrap().to_path_buf();
        fs::set_permissions(&folder, Permissions::from_mode(READ_ONLY_FOLDER)).unwrap();
        let refused = inside.check_install(COMMAND);
        fs::set_permissions(&folder, Permissions::from_mode(OWNER_FOLDER)).unwrap();

        assert!(
            matches!(refused, Err(HookFileError::ReadOnly { .. })),
            "{refused:?}"
        );
    }

    /// A link is written through, a link to a file not made yet included,
    /// and stays a link.
    #[test]
    fn a_link_stays_a_link() {
        let (file, scratch) = scratch_file();
        let real = scratch.path().join("dotfiles").join("settings.json");
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

        assert!(file.uninstall().unwrap());
        assert!(
            fs::symlink_metadata(file.path())
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert!(real.exists());
    }

    /// A file another writer changes between muster's read and its write
    /// is read again, so that writer's change is kept.
    #[test]
    fn a_change_made_during_the_edit_is_kept() {
        let (file, _scratch) = scratch_file();
        holding(&file, NO_HOOKS);
        let raced = NO_HOOKS.replace("\"zeta\": 1", "\"zeta\": 2");
        let first = Cell::new(true);

        let wrote = file
            .apply(|text, path| {
                if first.replace(false) {
                    fs::write(path, &raced).unwrap();
                }
                installed(text, COMMAND, path)
            })
            .unwrap();

        assert!(wrote);
        assert_eq!(json_of(&file)["zeta"], 2);
        assert_eq!(
            json_of(&file)["hooks"]["SessionStart"][0],
            hook_group(COMMAND)
        );
    }
}
