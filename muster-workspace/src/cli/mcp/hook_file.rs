//! An agent's hooks file (Claude Code's `settings.json`, Codex's
//! `hooks.json`), into which `muster mcp add` puts a session-start hook
//! running `muster mcp context`, and from which `muster mcp remove` takes it.
//! The file is edited as text through its syntax tree, so every key, its
//! order, the indentation and any comment stay byte for byte as they were.

use std::{
    fs,
    io::{self, ErrorKind},
    path::{Path, PathBuf},
};

use getset::Getters;
use jsonc_parser::{
    ParseOptions,
    cst::{CstArray, CstInputValue, CstNode, CstObject, CstRootNode},
};
use serde_json::{Value, json};
use typed_builder::TypedBuilder;

use crate::cli::{
    mcp::{
        agent_keys::{
            COMMAND_HANDLER, COMMAND_KEY, HOOKS_KEY, MATCHER_KEY, SESSION_START_KEY, TIMEOUT_KEY,
            TYPE_KEY,
        },
        hook_command::is_context_command,
        hook_edit::HookEdit,
        hook_file_error::HookFileError,
        hook_file_part::HookFilePart,
        json_shape::JsonShape,
    },
    transfer::text_file::write_atomically,
};

/// The session sources the hook runs on: a new session, a resumed one, one
/// cleared and one compacted, each of which starts from a fresh context.
pub const SESSION_SOURCES: &str = "^(startup|resume|clear|compact)$";
/// How long the agent lets the hook run, in seconds; it only reads the
/// database, and gives up itself well before this.
pub const HOOK_TIMEOUT_S: u64 = 10;
/// How many times an edit starts over when the file changed under it.
const EDIT_ATTEMPTS: usize = 2;
/// The most links followed to the file a hooks path names, as the kernel
/// allows.
const MAX_LINK_HOPS: usize = 40;
/// Why a chain of links was not followed to its end.
const TOO_MANY_LINKS: &str = "too many levels of symbolic links";
/// What a file holding only an empty object reads as, whitespace aside.
const EMPTY_OBJECT: &str = "{}";

/// An edit of a hooks file's text, or the refusal of it.
type Edited = Result<Option<HookEdit>, HookFileError>;

/// One agent's hooks file, which may not exist yet.
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct HookFile {
    /// Where the agent reads it.
    path: PathBuf,
}

impl HookFile {
    /// Whether the file holds muster's hook; an absent one does not.
    ///
    /// # Errors
    /// Returns [`HookFileError`] when the file cannot be read or parsed.
    pub fn has_hook(&self) -> Result<bool, HookFileError> {
        let Some(text) = self.read_text()? else {
            return Ok(false);
        };
        let root = parsed(&text, &self.path)?;
        Ok(session_groups(&root, &self.path)?
            .is_some_and(|groups| groups.elements().iter().any(holds_muster_handler)))
    }

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
    /// read-only, is not JSON, holds its hooks in another shape, or kept
    /// changing while muster edited it; it is then untouched.
    pub fn install(&self, command: &str) -> Result<bool, HookFileError> {
        self.apply(|text, path| installed(text, command, path))
    }

    /// Takes muster's session-start hook out of the file, and the group,
    /// event and hooks entries that leaves empty; a file left holding only
    /// `{}` is deleted, unless it is a link. Returns whether the file
    /// changed; an absent file stays absent.
    ///
    /// # Errors
    /// Returns [`HookFileError`] as [`Self::install`] does.
    pub fn uninstall(&self) -> Result<bool, HookFileError> {
        self.apply(uninstalled)
    }

    /// Whether `edit` would change the file, refusing what it refuses and a
    /// read-only file it would change.
    ///
    /// # Errors
    /// Returns the [`HookFileError`] of `edit`, of a read, or
    /// [`HookFileError::ReadOnly`].
    fn check(&self, edit: impl Fn(Option<&str>, &Path) -> Edited) -> Result<bool, HookFileError> {
        let text = self.read_text()?;
        if edit(text.as_deref(), &self.path)?.is_none() {
            return Ok(false);
        }
        writable(&self.target()?)?;
        Ok(true)
    }

    /// Reads the file, edits it with `edit` and writes the result, starting
    /// over when the file changed between the read and the write so no
    /// other writer's change is lost. Returns whether it wrote.
    ///
    /// # Errors
    /// Returns the [`HookFileError`] of `edit`, of a read or a write,
    /// [`HookFileError::ReadOnly`], or [`HookFileError::Changed`] when the
    /// file changed on every attempt.
    fn apply(&self, edit: impl Fn(Option<&str>, &Path) -> Edited) -> Result<bool, HookFileError> {
        for _ in 0..EDIT_ATTEMPTS {
            let before = self.read_text()?;
            let Some(edited) = edit(before.as_deref(), &self.path)? else {
                return Ok(false);
            };
            let target = self.target()?;
            writable(&target)?;
            if self.read_text()? != before {
                continue;
            }
            self.commit(&target, edited)?;
            return Ok(true);
        }
        Err(HookFileError::Changed {
            path: self.path.clone(),
        })
    }

    /// Writes `edited` to `target`, the file the path names: deleted when
    /// emptied, unless the path is a link.
    ///
    /// # Errors
    /// Returns [`HookFileError::Write`] when the folder or file cannot be
    /// written or removed.
    fn commit(&self, target: &Path, edited: HookEdit) -> Result<(), HookFileError> {
        let failed = |source| HookFileError::Write {
            path: self.path.clone(),
            source,
        };
        let linked =
            fs::symlink_metadata(&self.path).is_ok_and(|meta| meta.file_type().is_symlink());
        let text = match edited {
            HookEdit::Emptied(_) if !linked => return fs::remove_file(target).map_err(failed),
            HookEdit::Emptied(text) | HookEdit::Write(text) => text,
        };
        if let Some(folder) = target.parent() {
            fs::create_dir_all(folder).map_err(failed)?;
        }
        write_atomically(target, &text).map_err(failed)
    }

    /// The file's text; none when it does not exist, a link to nothing
    /// included.
    ///
    /// # Errors
    /// Returns [`HookFileError::Read`] when it cannot be read.
    fn read_text(&self) -> Result<Option<String>, HookFileError> {
        match fs::read_to_string(&self.path) {
            Ok(text) => Ok(Some(text)),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(source) => Err(HookFileError::Read {
                path: self.path.clone(),
                source,
            }),
        }
    }

    /// The file the path names once every link is followed, even to a
    /// target that does not exist yet, so a linked dotfile stays linked.
    ///
    /// # Errors
    /// Returns [`HookFileError::Read`] when a link cannot be read or the
    /// chain does not end.
    fn target(&self) -> Result<PathBuf, HookFileError> {
        let failed = |source| HookFileError::Read {
            path: self.path.clone(),
            source,
        };
        let mut path = self.path.clone();
        for _ in 0..MAX_LINK_HOPS {
            match fs::symlink_metadata(&path) {
                Ok(meta) if meta.file_type().is_symlink() => {
                    let link = fs::read_link(&path).map_err(failed)?;
                    path = match path.parent() {
                        Some(folder) => folder.join(link),
                        None => link,
                    };
                },
                _ => return Ok(path),
            }
        }
        Err(failed(io::Error::other(TOO_MANY_LINKS)))
    }
}

/// Refuses an existing `target` its owner made read-only.
///
/// # Errors
/// Returns [`HookFileError::ReadOnly`] for such a file.
fn writable(target: &Path) -> Result<(), HookFileError> {
    match fs::metadata(target) {
        Ok(meta) if meta.permissions().readonly() => Err(HookFileError::ReadOnly {
            path: target.to_path_buf(),
        }),
        _ => Ok(()),
    }
}

/// `text`, the file at `path`, as a syntax tree.
///
/// # Errors
/// Returns [`HookFileError::Malformed`] when it is not JSON.
fn parsed(text: &str, path: &Path) -> Result<CstRootNode, HookFileError> {
    CstRootNode::parse(text, &ParseOptions::default()).map_err(|error| HookFileError::Malformed {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

/// The refusal of `part` of the file at `path` for not being `shape`.
fn unexpected(path: &Path, part: HookFilePart, shape: JsonShape) -> HookFileError {
    HookFileError::Unexpected {
        path: path.to_path_buf(),
        part,
        shape,
    }
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

/// The session-start groups of `root`, the file at `path`; none when it has
/// none.
///
/// # Errors
/// Returns [`HookFileError::Unexpected`] when its top level, hooks or
/// session-start entry has another shape.
fn session_groups(root: &CstRootNode, path: &Path) -> Result<Option<CstArray>, HookFileError> {
    let Some(object) = top_object(root, path)? else {
        return Ok(None);
    };
    let Some(hooks) = object.get(HOOKS_KEY) else {
        return Ok(None);
    };
    let hooks = hooks
        .object_value()
        .ok_or_else(|| unexpected(path, HookFilePart::Hooks, JsonShape::Object))?;
    let Some(groups) = hooks.get(SESSION_START_KEY) else {
        return Ok(None);
    };
    groups
        .array_value()
        .map(Some)
        .ok_or_else(|| unexpected(path, HookFilePart::SessionStart, JsonShape::List))
}

/// The hook group muster adds: its sources and one handler running
/// `command`, as the file holds it.
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

/// The same group, as the syntax tree takes it, its keys in this order.
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
/// Returns [`HookFileError::Malformed`] or [`HookFileError::Unexpected`].
fn installed(text: Option<&str>, command: &str, path: &Path) -> Edited {
    let root = parsed(text.unwrap_or_default(), path)?;
    let object = match top_object(&root, path)? {
        Some(object) => object,
        None => root.object_value_or_set(),
    };
    let hooks = match object.get(HOOKS_KEY) {
        Some(entry) => entry
            .object_value()
            .ok_or_else(|| unexpected(path, HookFilePart::Hooks, JsonShape::Object))?,
        None => object.object_value_or_set(HOOKS_KEY),
    };
    let groups = match hooks.get(SESSION_START_KEY) {
        Some(entry) => entry
            .array_value()
            .ok_or_else(|| unexpected(path, HookFilePart::SessionStart, JsonShape::List))?,
        None => hooks.array_value_or_set(SESSION_START_KEY),
    };
    let mine: Vec<CstNode> = groups
        .elements()
        .into_iter()
        .filter(holds_muster_handler)
        .collect();
    let wanted = hook_group(command);
    if let [only] = mine.as_slice()
        && only.to_serde_value().as_ref() == Some(&wanted)
    {
        return Ok(None);
    }
    strip_muster_handlers(&groups);
    groups.append(hook_group_input(command));
    Ok(Some(HookEdit::Write(root.to_string())))
}

/// `text`, the file at `path`, with every muster handler taken out, and the
/// group, event and hooks entries that leaves empty; none when it holds
/// none.
///
/// # Errors
/// Returns [`HookFileError::Malformed`] or [`HookFileError::Unexpected`].
fn uninstalled(text: Option<&str>, path: &Path) -> Edited {
    let Some(text) = text else {
        return Ok(None);
    };
    let root = parsed(text, path)?;
    let Some(groups) = session_groups(&root, path)? else {
        return Ok(None);
    };
    if !strip_muster_handlers(&groups) {
        return Ok(None);
    }
    if let Some(object) = root.object_value()
        && let Some(hooks_entry) = object.get(HOOKS_KEY)
        && let Some(hooks) = hooks_entry.object_value()
    {
        if groups.elements().is_empty()
            && let Some(event) = hooks.get(SESSION_START_KEY)
        {
            event.remove();
        }
        if hooks.properties().is_empty() {
            hooks_entry.remove();
        }
    }
    let edited = root.to_string();
    let bare = edited
        .chars()
        .filter(|character| !character.is_whitespace())
        .eq(EMPTY_OBJECT.chars());
    Ok(Some(if bare {
        HookEdit::Emptied(edited)
    } else {
        HookEdit::Write(edited)
    }))
}

#[cfg(test)]
mod tests {
    use std::{
        cell::Cell,
        fs::Permissions,
        os::unix::fs::{PermissionsExt, symlink},
    };

    use tempfile::TempDir;

    use super::*;

    /// The command muster's hook runs in these tests.
    const COMMAND: &str = "/opt/bin/muster mcp context";
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
    /// How deep a hook handler's keys sit in those files: six levels of
    /// their 4-space indent.
    const HANDLER_INDENT: usize = 24;
    /// The mode of a file nobody may write.
    const READ_ONLY_MODE: u32 = 0o444;

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
        CstRootNode::parse(&text, &ParseOptions::default())
            .unwrap()
            .to_serde_value()
            .unwrap()
    }

    /// An absent file and its folder are made holding only the hook, and
    /// taking it out deletes the file again.
    #[test]
    fn an_absent_file_gets_only_the_hook_and_goes_with_it() {
        let (file, _scratch) = scratch_file();

        assert!(file.check_install(COMMAND).unwrap());
        assert!(!file.path().exists(), "a check writes nothing");
        assert!(file.install(COMMAND).unwrap());
        assert_eq!(
            json_of(&file),
            json!({"hooks": {"SessionStart": [hook_group(COMMAND)]}})
        );

        assert!(file.uninstall().unwrap());
        assert!(!file.path().exists());
    }

    /// An empty file reads as an empty object.
    #[test]
    fn an_empty_file_gets_the_hook() {
        let (file, _scratch) = scratch_file();
        holding(&file, "");

        assert!(file.install(COMMAND).unwrap());

        assert_eq!(
            json_of(&file)["hooks"]["SessionStart"][0],
            hook_group(COMMAND)
        );
    }

    /// Adding then removing on a file kept by hand gives back its exact
    /// bytes, its order, indentation and comments included, and the added
    /// hook follows the file's own indentation.
    #[test]
    fn add_then_remove_gives_back_the_same_bytes() {
        for original in [HAND_KEPT, NO_HOOKS] {
            let (file, _scratch) = scratch_file();
            holding(&file, original);

            assert!(file.install(COMMAND).unwrap());
            let added = fs::read_to_string(file.path()).unwrap();
            let indent = " ".repeat(HANDLER_INDENT);
            assert!(
                added.contains(&format!("\n{indent}\"command\": \"{COMMAND}\",\n")),
                "the file's own indent: {added}"
            );
            assert!(file.uninstall().unwrap());

            assert_eq!(fs::read_to_string(file.path()).unwrap(), original);
        }
    }

    /// Adding again writes nothing, and adding from a moved binary replaces
    /// the hook rather than adding a second.
    #[test]
    fn adding_again_never_duplicates_the_hook() {
        let (file, _scratch) = scratch_file();
        holding(&file, HAND_KEPT);
        assert!(file.install(COMMAND).unwrap());
        let written = fs::read_to_string(file.path()).unwrap();

        assert!(!file.check_install(COMMAND).unwrap());
        assert!(!file.install(COMMAND).unwrap());
        assert_eq!(fs::read_to_string(file.path()).unwrap(), written);
        assert!(file.has_hook().unwrap());

        let moved = "'/home/me/my tools/muster' mcp context";
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

    /// A file that is not JSON, or whose hooks are another shape, is
    /// refused by the check and the edit alike and left byte for byte.
    #[test]
    fn a_malformed_file_is_refused_unwritten() {
        let (file, _scratch) = scratch_file();
        for text in [
            "{\"hooks\": ",
            "[1, 2]",
            r#"{"hooks": []}"#,
            r#"{"hooks": {"SessionStart": {}}}"#,
        ] {
            holding(&file, text);

            assert!(file.check_install(COMMAND).is_err(), "{text}");
            assert!(file.install(COMMAND).is_err(), "{text}");
            assert!(file.check_uninstall().is_err(), "{text}");
            assert!(file.uninstall().is_err(), "{text}");
            assert!(file.has_hook().is_err(), "{text}");
            assert_eq!(fs::read_to_string(file.path()).unwrap(), text);
        }
    }

    /// A read-only file is refused, by the check too, and left as it was.
    #[test]
    fn a_read_only_file_is_refused() {
        let (file, _scratch) = scratch_file();
        holding(&file, NO_HOOKS);
        fs::set_permissions(file.path(), Permissions::from_mode(READ_ONLY_MODE)).unwrap();

        assert!(matches!(
            file.check_install(COMMAND),
            Err(HookFileError::ReadOnly { .. })
        ));
        assert!(matches!(
            file.install(COMMAND),
            Err(HookFileError::ReadOnly { .. })
        ));
        assert_eq!(fs::read_to_string(file.path()).unwrap(), NO_HOOKS);
    }

    /// A link is written through, a link to a file not made yet included,
    /// and stays a link even once emptied.
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
