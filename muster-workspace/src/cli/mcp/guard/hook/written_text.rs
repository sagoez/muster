//! What a file reads once Claude Code's write or edit lands, rebuilt from
//! the call so the guard can judge a memory file before it is written.

use std::{io::ErrorKind, path::Path};

use muster_os::bounded_read::read_bounded;
use serde_json::Value;

use crate::cli::mcp::guard::hook::hook_tool::HookTool;

/// The key of a write's whole new text.
const CONTENT_KEY: &str = "content";
/// The key of a multi-edit's list of edits.
const EDIT_LIST_KEY: &str = "edits";
/// The key of the text an edit replaces.
const OLD_STRING_KEY: &str = "old_string";
/// The key of the text an edit puts in its place.
const NEW_STRING_KEY: &str = "new_string";
/// The key asking an edit to replace every occurrence.
const REPLACE_ALL_KEY: &str = "replace_all";
/// The most bytes of a file the guard reads or rebuilds to judge a call:
/// far past any memory file, short of costing the hook its deadline.
const WRITTEN_MAX_BYTES: usize = 1024 * 1024;
/// The most bytes of text a multi-edit's edits may go over between them,
/// each counted for the text it is applied to: far past any memory file's
/// edits, and milliseconds of work, so no list of edits costs the hook its
/// deadline.
const REPLAY_MAX_BYTES: usize = 16 * 1024 * 1024;
/// How many matches show an edit's text is not unique: one, and another.
const ONE_AND_ANOTHER: usize = 2;
/// The text forms Claude Code reads as one when it matches an edit: a
/// Windows line ending and curly quotes, each with the form it matches as.
const MATCHED_AS: [(&str, &str); 5] = [
    ("\r\n", "\n"),
    ("\u{2018}", "'"),
    ("\u{2019}", "'"),
    ("\u{201c}", "\""),
    ("\u{201d}", "\""),
];

impl HookTool {
    /// The text the file at `path` holds once Claude Code's write or edit
    /// with `input` lands, in the plain form [`MATCHED_AS`] gives: a write's
    /// content, or an edit's replacements applied to the file as it is now,
    /// a missing file reading as empty, as an edit with an empty
    /// `old_string` creates one. None for other tools, for what is not a
    /// regular file, for an edit Claude Code would refuse, and past
    /// [`WRITTEN_MAX_BYTES`] or [`REPLAY_MAX_BYTES`], so the guard never
    /// reads, builds or replays without bound.
    pub fn written(self, input: &Value, path: &Path) -> Option<String> {
        let text = match self {
            Self::Write => plain(input.get(CONTENT_KEY)?.as_str()?),
            Self::Edit => replaced(current_text(path)?, input)?,
            Self::MultiEdit => {
                replayed(current_text(path)?, input.get(EDIT_LIST_KEY)?.as_array()?)?
            },
            Self::Bash | Self::NotebookEdit | Self::ApplyPatch => return None,
        };
        (text.len() <= WRITTEN_MAX_BYTES).then_some(text)
    }
}

/// `text` with each of a multi-edit's `edits` applied in turn, as
/// [`replaced`] applies one; none when one is refused, or once the edits
/// would go over more than [`REPLAY_MAX_BYTES`] of text between them.
fn replayed(mut text: String, edits: &[Value]) -> Option<String> {
    let mut gone_over: usize = 0;
    for edit in edits {
        gone_over = gone_over.checked_add(text.len().max(1))?;
        if gone_over > REPLAY_MAX_BYTES {
            return None;
        }
        text = replaced(text, edit)?;
    }
    Some(text)
}

/// The text of the file at `path` as it is now, in plain form: empty when
/// there is none; none when it is not a regular file, is larger than
/// [`WRITTEN_MAX_BYTES`] or does not read. It is opened without blocking
/// and checked on the open handle, so a FIFO is never waited on.
fn current_text(path: &Path) -> Option<String> {
    match read_bounded(path, WRITTEN_MAX_BYTES as u64) {
        Ok(text) => Some(plain(&text)),
        Err(error) if error.kind() == ErrorKind::NotFound => Some(String::new()),
        Err(_) => None,
    }
}

/// `text` with each form [`MATCHED_AS`] lists read as the one it matches as.
fn plain(text: &str) -> String {
    MATCHED_AS
        .iter()
        .fold(text.to_owned(), |text, (form, matched)| {
            text.replace(form, matched)
        })
}

/// `text` with the replacement `edit` describes, as Claude Code applies it,
/// in plain form: the one occurrence, or every one with `replace_all`; an
/// empty `old_string` only fills an empty file. None for an edit Claude
/// Code refuses (a missing text, no match, several without `replace_all`,
/// or nothing changed), and when the result could pass
/// [`WRITTEN_MAX_BYTES`].
fn replaced(text: String, edit: &Value) -> Option<String> {
    let old = plain(edit.get(OLD_STRING_KEY)?.as_str()?);
    let new = plain(edit.get(NEW_STRING_KEY)?.as_str()?);
    if old == new {
        return None;
    }
    if old.is_empty() {
        return text.is_empty().then_some(new);
    }
    let every = edit
        .get(REPLACE_ALL_KEY)
        .and_then(Value::as_bool)
        .unwrap_or_default();
    let count = if every {
        text.matches(&old).count()
    } else {
        text.matches(&old).take(ONE_AND_ANOTHER).count()
    };
    let most = text.len().checked_add(count.checked_mul(new.len())?)?;
    if count == 0 || (count > 1 && !every) || most > WRITTEN_MAX_BYTES {
        return None;
    }
    Some(text.replace(&old, &new))
}

#[cfg(test)]
mod tests {
    use std::{fs, path::PathBuf};

    use muster_os::test_fifo::make_fifo;
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    /// A file holding `text` in a fresh scratch folder, returned with the
    /// folder so it lives as long as the test.
    fn file_holding(text: &str) -> (TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("a.md");
        fs::write(&path, text).unwrap();
        (dir, path)
    }

    /// A write is its content; an edit replaces its one occurrence, or
    /// every one with `replace_all`; a multi-edit applies its edits in
    /// order; a shell command or a patch writes nothing judged.
    #[test]
    fn written_is_the_file_after_the_call() {
        let (_dir, path) = file_holding("a b a");
        assert_eq!(
            HookTool::Write.written(&json!({"content": "new"}), &path),
            Some("new".to_owned())
        );
        let edit = json!({"old_string": "b", "new_string": "c"});
        assert_eq!(
            HookTool::Edit.written(&edit, &path),
            Some("a c a".to_owned())
        );
        let every = json!({"old_string": "a", "new_string": "c", "replace_all": true});
        assert_eq!(
            HookTool::Edit.written(&every, &path),
            Some("c b c".to_owned())
        );
        let chain = json!({"edits": [
            {"old_string": "b", "new_string": "x"},
            {"old_string": "x", "new_string": "y"},
        ]});
        assert_eq!(
            HookTool::MultiEdit.written(&chain, &path),
            Some("a y a".to_owned())
        );
        assert_eq!(HookTool::Bash.written(&edit, &path), None);
        assert_eq!(HookTool::ApplyPatch.written(&edit, &path), None);
    }

    /// An edit Claude Code refuses is not judged: its text missing, matching
    /// nothing, matching more than once without `replace_all`, or changing
    /// nothing.
    #[test]
    fn a_refused_edit_is_not_judged() {
        let (_dir, path) = file_holding("a b a");
        for refused in [
            json!({"new_string": "x"}),
            json!({"old_string": "absent", "new_string": "x"}),
            json!({"old_string": "a", "new_string": "x"}),
            json!({"old_string": "b", "new_string": "b"}),
        ] {
            assert_eq!(HookTool::Edit.written(&refused, &path), None, "{refused}");
        }
    }

    /// An edit with an empty `old_string` creates a missing file with its
    /// new text, and does nothing judged to a file holding text.
    #[test]
    fn an_empty_old_string_only_creates_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("new.md");
        let create = json!({"old_string": "", "new_string": "fact"});
        assert_eq!(
            HookTool::Edit.written(&create, &missing),
            Some("fact".to_owned())
        );
        assert_eq!(
            HookTool::MultiEdit.written(&json!({"edits": [create]}), &missing),
            Some("fact".to_owned())
        );
        let (_dir, path) = file_holding("text");
        assert_eq!(HookTool::Edit.written(&create, &path), None);
    }

    /// No result past [`WRITTEN_MAX_BYTES`] is built, however the edits
    /// grow it, no file past it is read, no multi-edit going over more than
    /// [`REPLAY_MAX_BYTES`] is replayed, and a FIFO is never opened.
    #[test]
    fn written_never_grows_or_reads_without_bound() {
        let (_dir, path) = file_holding("a");
        let double = json!({"old_string": "a", "new_string": "aa", "replace_all": true});
        let doubling = json!({"edits": vec![double; 64]});
        assert_eq!(HookTool::MultiEdit.written(&doubling, &path), None);

        let (_dir, half) = file_holding(&"a".repeat(WRITTEN_MAX_BYTES / 2));
        let flip = json!({"old_string": "a", "new_string": "b", "replace_all": true});
        let flop = json!({"old_string": "b", "new_string": "a", "replace_all": true});
        let replays = REPLAY_MAX_BYTES / (WRITTEN_MAX_BYTES / 2) + 2;
        let endless: Vec<Value> = [flip, flop].into_iter().cycle().take(replays).collect();
        assert_eq!(
            HookTool::MultiEdit.written(&json!({"edits": endless}), &half),
            None
        );

        let (_dir, large) = file_holding(&"a".repeat(WRITTEN_MAX_BYTES + 1));
        let edit = json!({"old_string": "b", "new_string": "c"});
        assert_eq!(HookTool::Edit.written(&edit, &large), None);

        let dir = tempfile::tempdir().unwrap();
        let fifo = dir.path().join("fifo.md");
        make_fifo(&fifo);
        assert_eq!(HookTool::Edit.written(&edit, &fifo), None);

        let huge = "a".repeat(WRITTEN_MAX_BYTES + 1);
        assert_eq!(
            HookTool::Write.written(&json!({"content": huge}), &fifo),
            None
        );
    }

    /// An edit matches across CRLF line endings and curly quotes, as Claude
    /// Code matches it.
    #[test]
    fn an_edit_matches_as_claude_code_matches_it() {
        let (_dir, path) = file_holding("---\r\ntype: \u{201c}feedback\u{201d}\r\n---\r\n");
        let retype = json!({
            "old_string": "type: \"feedback\"\n---",
            "new_string": "type: project\n---",
        });
        assert_eq!(
            HookTool::Edit.written(&retype, &path),
            Some("---\ntype: project\n---\n".to_owned())
        );
    }
}
