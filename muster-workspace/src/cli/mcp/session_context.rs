//! `muster mcp context`: a project's notes and open todos, as the plain text
//! an agent's session-start hook adds to its context.

use muster_domain::{
    config::ConfigError,
    coordination::{Scratchpad, Todo, TodoStatus},
    port::CoordinationStore,
    project::Project,
    value::{EpochSeconds, printable},
};
use muster_os::sanitize::printable_lines;

use crate::cli::{coordination::todo::claim_note, rows::report::attribution};

/// The most characters the context holds, well under the 10,000 Claude Code
/// keeps of a hook's output and the roughly 2,500 tokens Codex shows whole.
pub const CONTEXT_MAX_CHARS: usize = 6_000;
/// Opens the context, before the project's name.
const LEAD: &str = "muster project";
/// Follows the project's name and folder: what the context is and what to
/// do with it.
const PURPOSE: &str = "Other agents and the human share notes and todos here. Read them in \
    full with muster's note_read and todo_list tools, claim a todo with todo_claim before \
    working it, and write what the next agent should know with note_write.";
/// Said when the project holds neither.
const NOTHING_YET: &str = "No notes or open todos yet.";
/// Heads the open todos.
const TODOS_HEADING: &str = "Open todos:";
/// Heads the notes.
const NOTES_HEADING: &str = "Notes:";
/// Opens each listed todo.
const ITEM: &str = "- ";
/// Opens each note's heading line.
const NOTE_MARK: &str = "## ";
/// Ends a context cut at [`CONTEXT_MAX_CHARS`].
const TRUNCATED: &str = "\n[cut short; note_read and todo_list have the rest]";

/// The context of `project` at `now`: its open todos, then its notes with
/// their bodies, as plain lines with every control character escaped, cut to
/// at most [`CONTEXT_MAX_CHARS`] characters.
///
/// # Errors
/// Returns the [`ConfigError`] of a store that cannot be read.
pub fn session_context(
    store: &dyn CoordinationStore,
    project: &Project,
    now: EpochSeconds,
) -> Result<String, ConfigError> {
    let root = project.root();
    let open: Vec<Todo> = store
        .todos(root)?
        .into_iter()
        .filter(|todo| todo.effective_status(now) != TodoStatus::Done)
        .collect();
    let notes = store.scratchpads(root)?;
    let mut lines = vec![
        format!(
            "{LEAD} {} ({}). {PURPOSE}",
            printable(project.name().as_ref()),
            printable(&root.as_ref().display().to_string()),
        ),
        String::new(),
    ];
    if open.is_empty() && notes.is_empty() {
        lines.push(NOTHING_YET.to_owned());
    }
    if !open.is_empty() {
        lines.push(TODOS_HEADING.to_owned());
        lines.extend(open.iter().map(|todo| todo_line(todo, now)));
        lines.push(String::new());
    }
    if !notes.is_empty() {
        lines.push(NOTES_HEADING.to_owned());
        lines.extend(notes.iter().map(note_lines));
    }
    Ok(capped(lines.join("\n").trim_end()))
}

/// One open todo as a line: its id, status and title, who holds it and who
/// wrote it.
fn todo_line(todo: &Todo, now: EpochSeconds) -> String {
    format!(
        "{ITEM}{} [{}] {}{}{}",
        printable(todo.id().as_ref()),
        todo.effective_status(now),
        printable(todo.title().as_ref()),
        printable(&claim_note(todo, now)),
        printable(&attribution(todo.author())),
    )
}

/// One note as its key and author on a heading line, its body below.
fn note_lines(note: &Scratchpad) -> String {
    format!(
        "{NOTE_MARK}{}{}\n{}\n",
        printable(note.key().as_ref()),
        printable(&attribution(note.author())),
        printable_lines(note.body().trim_end()),
    )
}

/// `text` whole when it fits in [`CONTEXT_MAX_CHARS`], else cut to fit with
/// [`TRUNCATED`] after it.
fn capped(text: &str) -> String {
    if text.chars().count() <= CONTEXT_MAX_CHARS {
        return text.to_owned();
    }
    let kept = CONTEXT_MAX_CHARS.saturating_sub(TRUNCATED.chars().count());
    let mut cut: String = text.chars().take(kept).collect();
    cut.push_str(TRUNCATED);
    cut
}

#[cfg(test)]
mod tests {
    use std::fs;

    use muster_domain::{
        config::ImportMode,
        coordination::{Author, NewTodoTitle, NoteBody, ScratchpadKey, TodoChange, TodoDeps},
        port::{ProjectStore, StackStore},
        value::ProjectRoot,
    };
    use muster_os::clock::epoch_now;
    use muster_store::{
        SqliteStore,
        test_support::{fresh_store, spec},
    };
    use tempfile::TempDir;

    use super::*;

    /// What the snapshot shows in place of the scratch project's folder.
    const SNAPSHOT_ROOT: &str = "/work/shop";
    /// What the snapshot shows in place of a generated todo id.
    const SNAPSHOT_TODO_ID: &str = "<todo-id>";

    /// A fresh store holding one registered project, `shop`, and the scratch
    /// folder both live in.
    fn shop() -> (SqliteStore, Project, TempDir) {
        let (store, scratch) = fresh_store();
        let dir = scratch.path().join("shop");
        fs::create_dir_all(&dir).unwrap();
        let root = ProjectRoot::try_new(dir.canonicalize().unwrap()).unwrap();
        StackStore::import(&store, &root, &[spec("web", "true")], ImportMode::Strict).unwrap();
        let project = store.project_containing(root.as_ref()).unwrap().unwrap();
        (store, project, scratch)
    }

    /// A project with nothing shared yet says so after naming itself.
    #[test]
    fn an_empty_project_says_it_holds_nothing_yet() {
        let (store, project, _scratch) = shop();

        let context = session_context(&store, &project, epoch_now()).unwrap();

        assert!(context.starts_with("muster project shop ("), "{context}");
        assert!(context.ends_with(NOTHING_YET), "{context}");
    }

    /// Open todos and notes with their bodies are listed, a done todo is
    /// not, and a control character in a body is escaped.
    #[test]
    fn open_todos_and_notes_are_listed_and_done_ones_are_not() {
        let (store, project, _scratch) = shop();
        let root = project.root();
        let agent = Author::agent("claude");
        let open = store
            .add_todo(
                root,
                &agent,
                &NewTodoTitle::try_new("fix the login redirect").unwrap(),
                &TodoDeps::try_new(Vec::new()).unwrap(),
            )
            .unwrap();
        let done = store
            .add_todo(
                root,
                &Author::human(),
                &NewTodoTitle::try_new("ship the logo").unwrap(),
                &TodoDeps::try_new(Vec::new()).unwrap(),
            )
            .unwrap();
        store
            .update_todo(
                root,
                &Author::human(),
                done.id(),
                &TodoChange::builder()
                    .title(None)
                    .status(Some(TodoStatus::Done))
                    .force(false)
                    .build(),
            )
            .unwrap();
        store
            .set_scratchpad(
                root,
                &agent,
                &ScratchpadKey::try_new("build").unwrap(),
                &NoteBody::try_new("run `just ci`\nthen \u{1b}[2J push").unwrap(),
            )
            .unwrap();

        let context = session_context(&store, &project, epoch_now()).unwrap();

        assert!(
            context.contains(&format!(
                "- {} [pending] fix the login redirect  (by claude)",
                open.id()
            )),
            "{context}"
        );
        assert!(!context.contains("ship the logo"), "{context}");
        assert!(
            context.contains("## build  (by claude)\nrun `just ci`\nthen \\u{1b}[2J push"),
            "{context}"
        );
        insta::assert_snapshot!(
            context
                .replace(&root.as_ref().display().to_string(), SNAPSHOT_ROOT)
                .replace(open.id().as_ref(), SNAPSHOT_TODO_ID)
        );
    }

    /// Notes past the cap are cut to it, ending with where the rest is.
    #[test]
    fn a_long_context_is_cut_to_the_cap() {
        let (store, project, _scratch) = shop();
        for index in 0..20 {
            store
                .set_scratchpad(
                    project.root(),
                    &Author::human(),
                    &ScratchpadKey::try_new(format!("note-{index}")).unwrap(),
                    &NoteBody::try_new("x".repeat(1_000)).unwrap(),
                )
                .unwrap();
        }

        let context = session_context(&store, &project, epoch_now()).unwrap();

        assert_eq!(context.chars().count(), CONTEXT_MAX_CHARS);
        assert!(context.ends_with(TRUNCATED), "{context}");
    }
}
