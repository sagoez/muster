use serde_json::json;

use super::{
    answer::Answer, error::CliError, report::attribution, row::Row, row_kind::RowKind,
    todo_command::TodoCommand,
};
use crate::{
    adapter::{
        client::{PageSize, TodoPage, TodoView},
        clock::epoch_now,
    },
    domain::{
        coordination::{
            Author, ClaimState, NewTodoTitle, NewTodoTitleError, TODO_TITLE_MAX_CHARS, Todo,
            TodoChange, TodoId, TodoStatus, TodoUpdate,
        },
        port::CoordinationStore,
        value::{EpochSeconds, ProjectRoot},
    },
};

/// What a refusal calls a todo's title.
const TODO_TITLE: &str = "todo title";
/// Line printed when the project has no todos yet.
const EMPTY_NOTE: &str = "no todos; add one with `muster todo add <title>`";
/// Columns between the fields of a listed todo line.
const FIELD_GAP: &str = "  ";
/// Line printed under a page that more todos follow, before the cursor.
const MORE_NOTE: &str = "more follow; read on with `muster todo --cursor ";
/// The JSON key of the id a delete named.
const ID: &str = "id";
/// The JSON key of whether a delete found something.
const DELETED: &str = "deleted";

/// Runs a `muster todo` action against `store` for `project`, returning what
/// to print. Absent a subcommand, lists one page of `limit` todos after the
/// todo `cursor` names.
///
/// # Errors
/// Returns [`CliError`] when a title, id or cursor is invalid, a status
/// targets an unknown todo or one an agent holds, or the store cannot be read
/// or written.
pub fn todo(
    command: Option<TodoCommand>,
    limit: PageSize,
    cursor: Option<&str>,
    store: &dyn CoordinationStore,
    project: &ProjectRoot,
    author: &Author,
) -> Result<Answer, CliError> {
    match command {
        None => list(store, project, limit, cursor, epoch_now()),
        Some(TodoCommand::Add { title, deps }) => add(store, project, author, &title, &deps),
        Some(TodoCommand::Status { id, status, force }) => {
            set_status(store, project, author, &id, status, force)
        },
        Some(TodoCommand::Rm { id }) => remove(store, project, &id),
    }
}

/// Parses a user-supplied id into a validated [`TodoId`].
///
/// # Errors
/// Returns [`CliError::InvalidTodoId`] when the id is blank or whitespace-only.
fn parse_id(id: &str) -> Result<TodoId, CliError> {
    TodoId::try_new(id).map_err(|_| CliError::InvalidTodoId(id.to_string()))
}

/// Lists one page of `limit` todos for `project`, after the todo `cursor`
/// names, as `id  status  title`, one per line, with who holds it and for how
/// long at `now` (Unix seconds), and how to read on when more follow.
///
/// # Errors
/// Returns [`CliError::UnknownTodo`] for a cursor naming no todo, or a store
/// read failure.
fn list(
    store: &dyn CoordinationStore,
    project: &ProjectRoot,
    limit: PageSize,
    cursor: Option<&str>,
    now: EpochSeconds,
) -> Result<Answer, CliError> {
    let after = cursor.map(parse_id).transpose()?;
    let todos = store.todos(project)?;
    let start = TodoPage::resume(&todos, after.as_ref())
        .ok_or_else(|| CliError::UnknownTodo(cursor.unwrap_or_default().to_string()))?;
    let listed = &todos[start..];
    let page = TodoPage::of(listed.iter().map(|todo| TodoView::at(todo, now)), limit);
    let mut rows: Vec<Row> = listed
        .iter()
        .take(page.todos().len())
        .map(|todo| {
            Row::unlabeled(
                RowKind::Plain,
                format!(
                    "{id}{FIELD_GAP}{status}{FIELD_GAP}{title}{claim}{attribution}",
                    id = todo.id().as_ref(),
                    status = todo.effective_status(now),
                    title = todo.title().as_ref(),
                    claim = claim_note(todo, now),
                    attribution = attribution(todo.author()),
                ),
            )
        })
        .collect();
    if todos.is_empty() {
        rows.push(Row::unlabeled(RowKind::Hint, EMPTY_NOTE));
    }
    if let Some(next) = page.cursor() {
        rows.push(Row::unlabeled(
            RowKind::Hint,
            format!("{MORE_NOTE}{}`", next.as_ref()),
        ));
    }
    Answer::of(rows, &page)
}

/// Who holds `todo` at `now`, as a suffix for its listed line: the holder and
/// what its lease has left, or whose lease ran out.
fn claim_note(todo: &Todo, now: EpochSeconds) -> String {
    match todo.claim_state(now) {
        ClaimState::Unclaimed => String::new(),
        ClaimState::Held { owner, expires_in } => format!(
            "{FIELD_GAP}[claimed by {owner}, {} left]",
            humantime::format_duration(expires_in)
        ),
        ClaimState::Stale { owner } => format!("{FIELD_GAP}[{owner}'s claim lapsed]"),
    }
}

/// Adds a new pending todo with `title` and `deps`, printing its generated id.
///
/// # Errors
/// Returns [`CliError::InvalidTodoTitle`] for a blank title,
/// [`CliError::TooLong`] for one past [`TODO_TITLE_MAX_CHARS`],
/// [`CliError::InvalidTodoId`] for a blank dependency id, or a store failure.
fn add(
    store: &dyn CoordinationStore,
    project: &ProjectRoot,
    author: &Author,
    title: &str,
    deps: &[String],
) -> Result<Answer, CliError> {
    let title = NewTodoTitle::try_new(title).map_err(|error| match error {
        NewTodoTitleError::TooLong => CliError::TooLong {
            what: TODO_TITLE,
            max: TODO_TITLE_MAX_CHARS,
            found: title.chars().count(),
        },
        NewTodoTitleError::Blank => CliError::InvalidTodoTitle(title.to_string()),
    })?;
    let deps = deps
        .iter()
        .map(|dep| parse_id(dep))
        .collect::<Result<Vec<_>, _>>()?;
    let created = store.add_todo(project, author, &title, &deps)?;
    Answer::of(
        vec![Row::unlabeled(
            RowKind::Ok,
            created.id().as_ref().to_string(),
        )],
        &TodoView::at(&created, epoch_now()),
    )
}

/// Sets the status of the todo identified by `id`; `force` overrides an
/// agent's live claim.
///
/// # Errors
/// Returns [`CliError::InvalidTodoId`] for a blank id, [`CliError::UnknownTodo`]
/// when no todo has that id, [`CliError::HeldByOther`] when an agent holds it
/// and `force` is off, or a store failure.
fn set_status(
    store: &dyn CoordinationStore,
    project: &ProjectRoot,
    author: &Author,
    id: &str,
    status: TodoStatus,
    force: bool,
) -> Result<Answer, CliError> {
    let parsed = parse_id(id)?;
    let change = TodoChange::builder()
        .title(None)
        .status(Some(status))
        .force(force)
        .build();
    match store.update_todo(project, author, &parsed, &change)? {
        TodoUpdate::Updated(updated) => Answer::of(
            vec![Row::unlabeled(RowKind::Ok, format!("{parsed} is {status}"))],
            &TodoView::at(&updated, epoch_now()),
        ),
        TodoUpdate::HeldBy { owner, expires_in } => Err(CliError::HeldByOther {
            what: format!("todo '{parsed}'"),
            owner,
            expires_in,
        }),
        TodoUpdate::Missing => Err(CliError::UnknownTodo(id.to_string())),
    }
}

/// Deletes the todo identified by `id`. Deleting an absent todo is not an error.
///
/// # Errors
/// Returns [`CliError::InvalidTodoId`] for a blank id, or a store failure.
fn remove(
    store: &dyn CoordinationStore,
    project: &ProjectRoot,
    id: &str,
) -> Result<Answer, CliError> {
    let parsed = parse_id(id)?;
    let deleted = store.delete_todo(project, &parsed)?;
    let row = if deleted {
        Row::unlabeled(RowKind::Ok, format!("removed '{parsed}'"))
    } else {
        Row::unlabeled(RowKind::Hint, format!("no todo '{parsed}' to remove"))
    };
    Answer::of(vec![row], &json!({ ID: parsed.as_ref(), DELETED: deleted }))
}

#[cfg(test)]
mod tests {
    use std::{fs, time::Duration};

    use tempfile::TempDir;

    use super::*;
    use crate::{
        adapter::store::{
            SqliteStore,
            test_support::{fresh_store, spec},
        },
        domain::{
            coordination::{Claim, TodoLease},
            port::StackStore,
        },
    };

    /// A store with one registered project, and the scratch folder holding
    /// both.
    fn fixture() -> (SqliteStore, ProjectRoot, TempDir) {
        let (store, scratch) = fresh_store();
        let dir = scratch.path().join("shop");
        fs::create_dir_all(&dir).unwrap();
        let root = ProjectRoot::try_new(dir.canonicalize().unwrap()).unwrap();
        store
            .add_process(&root, &spec("web", "true"), &Author::human())
            .unwrap();
        (store, root, scratch)
    }

    /// The added id is the row detail, so a script can capture it directly.
    fn added_id(store: &SqliteStore, root: &ProjectRoot, title: &str) -> String {
        add(store, root, &Author::human(), title, &[])
            .unwrap()
            .rows()[0]
            .detail()
            .clone()
    }

    /// `id`'s todo claimed by `agent` with the default lease.
    fn claim(store: &SqliteStore, root: &ProjectRoot, agent: &str, id: &str) -> Todo {
        let claim = store
            .claim_todo(
                root,
                &Author::agent(agent),
                Some(&parse_id(id).unwrap()),
                TodoLease::DEFAULT,
            )
            .unwrap();
        let Claim::Claimed(todo) = claim else {
            panic!("the todo is free, got {claim:?}");
        };
        todo
    }

    /// The moment leases are read at.
    fn now() -> EpochSeconds {
        epoch_now()
    }

    /// Adding prints the id; listing shows it with the pending status and title.
    #[test]
    fn add_then_list() {
        let (store, root, _scratch) = fixture();
        let id = added_id(&store, &root, "wire the store");

        let listed = list(&store, &root, PageSize::default(), None, now()).unwrap();
        assert_eq!(listed.rows().len(), 1);
        assert_eq!(
            listed.rows()[0].detail(),
            &format!("{id}  pending  wire the store")
        );
        assert_eq!(listed.json()["todos"][0]["id"], id.as_str());
        assert_eq!(listed.json()["todos"][0]["status"], "pending");
    }

    /// A claimed todo lists its holder and lease; once the lease lapses it
    /// lists as pending, naming whose claim ran out.
    #[test]
    fn list_shows_the_claim_and_a_lapsed_one() {
        let (store, root, _scratch) = fixture();
        let id = added_id(&store, &root, "task");
        let todo = claim(&store, &root, "claude", &id);
        let until = *todo.claim().as_ref().unwrap().claimed_until();

        let held = list(
            &store,
            &root,
            PageSize::default(),
            None,
            EpochSeconds::new(until.into_inner() - 90),
        )
        .unwrap();
        assert_eq!(
            held.rows()[0].detail(),
            &format!(
                "{id}  in-progress  task  [claimed by claude, {} left]  (by claude)",
                humantime::format_duration(Duration::from_secs(90))
            )
        );
        let lapsed = list(
            &store,
            &root,
            PageSize::default(),
            None,
            until.after(Duration::from_secs(1)),
        )
        .unwrap();
        assert_eq!(
            lapsed.rows()[0].detail(),
            &format!("{id}  pending  task  [claude's claim lapsed]  (by claude)")
        );
    }

    /// Setting a status updates the listed line; an unknown id errors.
    #[test]
    fn status_updates_and_unknown_errors() {
        let (store, root, _scratch) = fixture();
        let id = added_id(&store, &root, "task");

        let ok = set_status(
            &store,
            &root,
            &Author::human(),
            &id,
            TodoStatus::Done,
            false,
        )
        .unwrap();
        assert_eq!(ok.rows()[0].kind(), RowKind::Ok);
        assert_eq!(ok.json()["status"], "done");
        assert!(
            list(&store, &root, PageSize::default(), None, now())
                .unwrap()
                .rows()[0]
                .detail()
                .contains("done")
        );

        let unknown = TodoId::generate().unwrap();
        assert!(matches!(
            set_status(
                &store,
                &root,
                &Author::human(),
                unknown.as_ref(),
                TodoStatus::Done,
                false
            ),
            Err(CliError::UnknownTodo(_))
        ));
    }

    /// The human is refused a todo an agent holds, naming the agent, until
    /// they force it.
    #[test]
    fn a_held_todo_needs_force() {
        let (store, root, _scratch) = fixture();
        let id = added_id(&store, &root, "task");
        claim(&store, &root, "claude", &id);

        let refused = set_status(
            &store,
            &root,
            &Author::human(),
            &id,
            TodoStatus::Done,
            false,
        );
        assert!(matches!(
            refused,
            Err(CliError::HeldByOther { ref owner, .. }) if owner == &Author::agent("claude")
        ));
        assert!(refused.unwrap_err().to_string().contains("--force"));

        set_status(&store, &root, &Author::human(), &id, TodoStatus::Done, true).unwrap();
        assert!(
            list(&store, &root, PageSize::default(), None, now())
                .unwrap()
                .rows()[0]
                .detail()
                .contains("done")
        );
    }

    /// Removing succeeds once, then reports absence idempotently.
    #[test]
    fn remove_is_idempotent() {
        let (store, root, _scratch) = fixture();
        let id = added_id(&store, &root, "task");

        let first = remove(&store, &root, &id).unwrap();
        assert_eq!(first.rows()[0].kind(), RowKind::Ok);
        assert_eq!(first.json()["deleted"], true);
        assert_eq!(
            remove(&store, &root, &id).unwrap().rows()[0].kind(),
            RowKind::Hint
        );
    }

    /// A blank id or title is rejected before the store is touched.
    #[test]
    fn blank_inputs_are_rejected() {
        let (store, root, _scratch) = fixture();
        assert!(matches!(
            add(&store, &root, &Author::human(), "   ", &[]),
            Err(CliError::InvalidTodoTitle(_))
        ));
        assert!(matches!(
            add(
                &store,
                &root,
                &Author::human(),
                &"x".repeat(TODO_TITLE_MAX_CHARS + 1),
                &[]
            ),
            Err(CliError::TooLong {
                max: TODO_TITLE_MAX_CHARS,
                ..
            })
        ));
        assert!(matches!(
            add(&store, &root, &Author::human(), "task", &["  ".to_string()]),
            Err(CliError::InvalidTodoId(_))
        ));
        assert!(matches!(
            remove(&store, &root, "  "),
            Err(CliError::InvalidTodoId(_))
        ));
    }

    /// A listing pages by cursor like `todo_list`, saying how to read on, and
    /// a cursor naming no todo is refused.
    #[test]
    fn list_pages_by_cursor() {
        let (store, root, _scratch) = fixture();
        let ids: Vec<String> = ["one", "two", "three"]
            .iter()
            .map(|title| added_id(&store, &root, title))
            .collect();
        let two = PageSize::try_new(2).unwrap();

        let first = list(&store, &root, two, None, now()).unwrap();
        assert_eq!(first.json()["todos"].as_array().unwrap().len(), 2);
        assert_eq!(first.json()["more"], true);
        assert_eq!(first.json()["cursor"], ids[1].as_str());
        assert!(
            first.rows()[2].detail().contains(&ids[1]),
            "{:?}",
            first.rows()
        );

        let rest = list(&store, &root, two, Some(&ids[1]), now()).unwrap();
        assert_eq!(rest.json()["todos"][0]["id"], ids[2].as_str());
        assert_eq!(rest.json()["more"], false);
        assert!(rest.json().get("cursor").is_none());
        assert_eq!(rest.rows().len(), 1);

        assert!(matches!(
            list(&store, &root, two, Some("no-such-todo"), now()),
            Err(CliError::UnknownTodo(_))
        ));
    }

    /// An empty store lists a hint rather than nothing.
    #[test]
    fn list_reports_empty() {
        let (store, root, _scratch) = fixture();
        assert_eq!(
            list(&store, &root, PageSize::default(), None, now())
                .unwrap()
                .rows()[0]
                .detail(),
            EMPTY_NOTE
        );
    }
}
