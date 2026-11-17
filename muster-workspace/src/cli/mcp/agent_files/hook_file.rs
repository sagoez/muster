//! An agent's hooks file (Claude Code's `settings.json`, Codex's
//! `hooks.json`), into which `muster mcp add` puts a session-start hook
//! running `muster mcp context`, and from which `muster mcp remove` takes it.
//! The file is edited as text through its syntax tree, so every key, its
//! order, the indentation and any comment stay byte for byte as they were.

use std::{
    cell::RefCell,
    path::{Path, PathBuf},
};

use getset::Getters;
use jsonc_parser::{
    ParseOptions,
    cst::{CstArray, CstInputValue, CstNode, CstObject, CstRootNode},
};
use typed_builder::TypedBuilder;

use crate::cli::mcp::{
    agent_file::{read_optional, write_through},
    agent_file_error::AgentFileError,
    agent_keys::{
        COMMAND_HANDLER, COMMAND_KEY, HOOKS_KEY, MATCHER_KEY, SESSION_START_KEY, TIMEOUT_KEY,
        TYPE_KEY,
    },
    hook_command::hook_binary,
    hook_file_error::HookFileError,
    hook_file_part::HookFilePart,
    hook_install::HookInstall,
    json_shape::JsonShape,
    session_source::SessionSource,
};

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
    /// Whether a folder muster makes for it is owner-only, as a user's own
    /// config folder is, rather than made as any project folder.
    private: bool,
}

impl HookFile {
    /// Puts muster's session-start hook running `command` last among the
    /// file's session-start groups, creating the file and its folder when
    /// absent. A muster hook already there is left as it is, however the
    /// user shaped it, unless the binary it runs is gone: then only its
    /// command is pointed at `command`, which loses nothing. Returns what it
    /// did, read from the one parse of the file: running it twice writes
    /// once.
    ///
    /// # Errors
    /// Returns [`HookFileError`] when the file cannot be read or written, is
    /// not JSON, holds its hooks in another shape or twice, or kept changing
    /// while muster edited it; it is then untouched.
    pub fn install(&self, command: &str) -> Result<HookInstall, HookFileError> {
        let outcome = RefCell::new(HookInstall::Present);
        self.apply(|text, path| installed(text, command, path, &outcome))?;
        Ok(outcome.into_inner())
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

    /// Reads the file, edits it with `edit` and writes the result. When the
    /// file changed between the read and the write it starts over, which
    /// narrows the window in which another writer's change is lost but
    /// cannot close it. Returns whether it wrote.
    ///
    /// # Errors
    /// Returns the [`HookFileError`] of `edit`, of a read or a write, or
    /// [`HookFileError::Changed`] when the file changed on every attempt.
    fn apply(&self, edit: impl Fn(Option<&str>, &Path) -> Edited) -> Result<bool, HookFileError> {
        for _ in 0..EDIT_ATTEMPTS {
            let before = read_optional(&self.path)?;
            let Some(edited) = edit(before.as_deref(), &self.path)? else {
                return Ok(false);
            };
            if read_optional(&self.path)? != before {
                continue;
            }
            write_through(&self.path, &edited, self.private)?;
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
/// Returns [`AgentFileError::Malformed`] when it is not JSON.
fn parsed(text: &str, path: &Path) -> Result<(CstRootNode, bool), HookFileError> {
    let unmarked = text.strip_prefix(BYTE_ORDER_MARK);
    let body = unmarked.unwrap_or(text);
    let body = if body.trim().is_empty() { "" } else { body };
    CstRootNode::parse(body, &ParseOptions::default())
        .map(|root| (root, unmarked.is_some()))
        .map_err(|error| {
            HookFileError::File(AgentFileError::Malformed {
                path: path.to_path_buf(),
                message: error.to_string(),
            })
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

/// The hook group running `command` as a person would add it by hand to a
/// hooks file's `hooks.SessionStart` list, keys in the order muster writes
/// them.
pub fn hook_group_text(command: &str) -> String {
    CstRootNode::parse("", &ParseOptions::default())
        .map(|root| {
            root.set_value(hook_group_input(command));
            root.to_string().trim_end().to_owned()
        })
        .unwrap_or_default()
}

/// The hook group muster adds, the one place its shape is written: its
/// sources and one handler running `command`, keys in the order the file
/// shows them.
fn hook_group_input(command: &str) -> CstInputValue {
    CstInputValue::Object(vec![
        (MATCHER_KEY.to_owned(), SessionSource::matcher().into()),
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

/// The binary `handler` runs when it is muster's hook, read with the one
/// hook parser.
fn muster_binary(handler: &CstNode) -> Option<String> {
    hook_binary(&command_of(handler)?)
}

/// The command line `handler` runs.
fn command_of(handler: &CstNode) -> Option<String> {
    handler
        .to_serde_value()?
        .get(COMMAND_KEY)?
        .as_str()
        .map(str::to_owned)
}

/// The handlers of `group`, none when it holds no list of them.
fn handlers(group: &CstNode) -> Vec<CstNode> {
    group
        .as_object()
        .and_then(|group| group.array_value(HOOKS_KEY))
        .map(|list| list.elements())
        .unwrap_or_default()
}

/// Every muster handler in `groups`, with the binary it runs.
fn muster_handlers(groups: &CstArray) -> Vec<(CstNode, String)> {
    groups
        .elements()
        .iter()
        .flat_map(handlers)
        .filter_map(|handler| muster_binary(&handler).map(|binary| (handler, binary)))
        .collect()
}

/// Takes every muster handler out of `groups`, and each group that leaves
/// with no handler. Returns whether any was taken.
fn strip_muster_handlers(groups: &CstArray) -> bool {
    let mut stripped = false;
    for group in groups.elements() {
        let listed = handlers(&group);
        let mine: Vec<CstNode> = listed
            .iter()
            .filter(|handler| muster_binary(handler).is_some())
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
/// running `command` put in, saying what it did in `outcome`. A muster hook
/// already there is left as it is, however the user shaped it, and the
/// text then needs no change, unless the binary it runs is gone: then its
/// command alone is pointed at `command`, which loses nothing.
///
/// # Errors
/// Returns [`AgentFileError::Malformed`], [`HookFileError::Unexpected`] or
/// [`HookFileError::Duplicate`].
fn installed(
    text: Option<&str>,
    command: &str,
    path: &Path,
    outcome: &RefCell<HookInstall>,
) -> Edited {
    let (root, marked) = parsed(text.unwrap_or_default(), path)?;
    let groups = match session_groups(&root, path)? {
        Some((_, groups)) => groups,
        None => root
            .object_value_or_set()
            .object_value_or_set(HOOKS_KEY)
            .array_value_or_set(SESSION_START_KEY),
    };
    let mine = muster_handlers(&groups);
    if mine.is_empty() {
        groups.append(hook_group_input(command));
        outcome.replace(HookInstall::Added);
        return Ok(Some(written(&root, marked)));
    }
    let gone: Vec<(CstNode, String)> = mine
        .into_iter()
        .filter(|(handler, binary)| {
            is_gone(binary) && command_of(handler).as_deref() != Some(command)
        })
        .collect();
    if gone.is_empty() {
        outcome.replace(HookInstall::Present);
        return Ok(None);
    }
    for (handler, _) in &gone {
        if let Some(property) = handler
            .as_object()
            .and_then(|handler| handler.get(COMMAND_KEY))
        {
            property.set_value(command.into());
        }
    }
    outcome.replace(HookInstall::Repaired(
        gone.into_iter().map(|(_, binary)| binary).collect(),
    ));
    Ok(Some(written(&root, marked)))
}

/// Whether `binary` is an absolute path to nothing: a muster since moved or
/// removed, whose hook prints nothing.
fn is_gone(binary: &str) -> bool {
    let path = Path::new(binary);
    path.is_absolute() && !path.exists()
}

/// `text`, the file at `path`, with every muster handler taken out, and the
/// group, event and hooks entries that leaves empty; none when it holds
/// none.
///
/// # Errors
/// Returns [`AgentFileError::Malformed`], [`HookFileError::Unexpected`] or
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

    use serde_json::{Value, json};
    use tempfile::TempDir;

    use super::*;

    /// The command muster's hook runs in these tests: the bare `muster`,
    /// which is never a path gone.
    const COMMAND: &str = "{ muster mcp context --hook || true; } 2>/dev/null";
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
        json!({
            "matcher": "^(startup|resume|clear|compact)$",
            "hooks": [{"type": "command", "command": command, "timeout": HOOK_TIMEOUT_S}],
        })
    }

    /// A hooks file at `.claude/settings.json` in a fresh scratch folder.
    fn scratch_file() -> (HookFile, TempDir) {
        let scratch = TempDir::new().unwrap();
        let file = HookFile::builder()
            .path(scratch.path().join(".claude").join("settings.json"))
            .private(false)
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

    /// The group printed to add by hand is the one muster writes.
    #[test]
    fn the_hook_to_add_by_hand_is_the_one_written() {
        let printed: Value = serde_json::from_str(&hook_group_text(COMMAND)).unwrap();

        assert_eq!(printed, hook_group(COMMAND));
    }

    /// An absent file and its folder are made holding only the hook; taking
    /// it out leaves the file, as an empty object.
    #[test]
    fn an_absent_file_gets_only_the_hook() {
        let (file, _scratch) = scratch_file();

        assert!(file.install(COMMAND).unwrap().changed());
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

            assert!(file.install(COMMAND).unwrap().changed());

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

            assert!(file.install(COMMAND).unwrap().changed());
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

    /// Adding again writes nothing, and adding from a muster moved elsewhere
    /// leaves the hook already there as it is rather than adding a second.
    #[test]
    fn adding_again_never_duplicates_the_hook() {
        let (file, _scratch) = scratch_file();
        holding(&file, HAND_KEPT);
        assert!(file.install(COMMAND).unwrap().changed());
        let written = fs::read_to_string(file.path()).unwrap();

        assert!(!file.install(COMMAND).unwrap().changed());
        assert_eq!(fs::read_to_string(file.path()).unwrap(), written);

        let moved = "{ '/home/me/my tools/muster' mcp context --hook || true; } 2>/dev/null";
        assert_eq!(file.install(moved).unwrap(), HookInstall::Present);
        assert_eq!(fs::read_to_string(file.path()).unwrap(), written);
    }

    /// A muster hook whose binary is gone is repaired in place: only its
    /// command is pointed at the muster asked for, the rest of the group as
    /// the user shaped it, since the dead path loses nothing.
    #[test]
    fn a_hook_whose_binary_is_gone_is_repaired_in_place() {
        let (file, scratch) = scratch_file();
        let gone = scratch.path().join("gone").join("muster");
        let dead = format!(
            "{{ {} mcp context --hook || true; }} 2>/dev/null",
            gone.display()
        );
        holding(
            &file,
            &json!({"hooks": {"SessionStart": [{
                "matcher": "^startup$",
                "hooks": [{"type": "command", "command": dead, "timeout": 30}],
            }]}})
            .to_string(),
        );

        assert_eq!(
            file.install(COMMAND).unwrap(),
            HookInstall::Repaired(vec![gone.display().to_string()])
        );

        assert_eq!(
            json_of(&file),
            json!({"hooks": {"SessionStart": [{
                "matcher": "^startup$",
                "hooks": [{"type": "command", "command": COMMAND, "timeout": 30}],
            }]}})
        );
        assert_eq!(file.install(COMMAND).unwrap(), HookInstall::Present);
    }

    /// A hook running another binary with muster's own arguments, as a
    /// planted or unrelated hook could, is not muster's: an add puts
    /// muster's hook beside it, and a remove takes out muster's alone.
    #[test]
    fn a_hook_of_another_binary_is_never_muster_s() {
        let (file, _scratch) = scratch_file();
        let foreign = "{ /usr/local/bin/evil mcp context --hook || true; } 2>/dev/null";
        let planted = json!({"hooks": {"SessionStart": [{"hooks": [
            {"type": "command", "command": foreign},
        ]}]}})
        .to_string();
        holding(&file, &planted);

        assert_eq!(file.install(COMMAND).unwrap(), HookInstall::Added);
        let groups = json_of(&file)["hooks"]["SessionStart"].clone();
        assert_eq!(groups.as_array().unwrap().len(), 2, "{groups}");
        assert!(file.uninstall().unwrap());
        assert_eq!(
            json_of(&file),
            serde_json::from_str::<Value>(&planted).unwrap(),
            "the foreign hook stays, alone"
        );
    }

    /// A muster hook the user shaped, its matcher narrowed and its timeout
    /// raised, is left as it is: an add only makes sure one is there.
    #[test]
    fn a_hook_the_user_shaped_is_kept() {
        let (file, _scratch) = scratch_file();
        let shaped = json!({"hooks": {"SessionStart": [{
            "matcher": "^startup$",
            "hooks": [{"type": "command", "command": COMMAND, "timeout": 30}],
        }]}})
        .to_string();
        holding(&file, &shaped);

        assert!(!file.install(COMMAND).unwrap().changed());

        assert_eq!(fs::read_to_string(file.path()).unwrap(), shaped);
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

        assert!(!file.uninstall().unwrap());
        assert_eq!(fs::read_to_string(file.path()).unwrap(), planted);
        assert!(file.install(COMMAND).unwrap().changed());
        let groups = json_of(&file)["hooks"]["SessionStart"].clone();
        assert_eq!(groups.as_array().unwrap().len(), 2, "{groups}");
    }

    /// A file that is not JSON, whose hooks are another shape, or that holds
    /// `hooks` or `SessionStart` twice is refused by an add and a remove
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

            assert!(file.install(COMMAND).is_err(), "{text}");
            assert!(file.uninstall().is_err(), "{text}");
            assert_eq!(fs::read_to_string(file.path()).unwrap(), text);
        }
    }

    /// A read-only file, or one in a folder this user cannot write, is
    /// refused and left as it was.
    #[test]
    fn a_file_this_user_cannot_write_is_refused() {
        let (file, _scratch) = scratch_file();
        holding(&file, NO_HOOKS);
        fs::set_permissions(file.path(), Permissions::from_mode(READ_ONLY_FILE)).unwrap();

        assert!(matches!(
            file.install(COMMAND),
            Err(HookFileError::File(AgentFileError::ReadOnly { .. }))
        ));
        assert_eq!(fs::read_to_string(file.path()).unwrap(), NO_HOOKS);

        let (inside, _other) = scratch_file();
        holding(&inside, NO_HOOKS);
        let folder = inside.path().parent().unwrap().to_path_buf();
        fs::set_permissions(&folder, Permissions::from_mode(READ_ONLY_FOLDER)).unwrap();
        let refused = inside.install(COMMAND);
        fs::set_permissions(&folder, Permissions::from_mode(OWNER_FOLDER)).unwrap();

        assert!(
            matches!(
                refused,
                Err(HookFileError::File(AgentFileError::ReadOnly { .. }))
            ),
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

        assert!(file.install(COMMAND).unwrap().changed());
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
                installed(text, COMMAND, path, &RefCell::new(HookInstall::Present))
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
