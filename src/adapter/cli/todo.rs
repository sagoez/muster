use std::path::Path;

use super::{
    args::TodoCommand,
    error::CliError,
    report::{Row, RowKind, attribution},
};
use crate::domain::{
    coordination::{Author, TodoId, TodoStatus, TodoTitle},
    port::CoordinationStore,
};

/// Line printed when the project has no todos yet.
const EMPTY_NOTE: &str = "no todos; add one with `muster todo add <title>`";
/// Columns between the fields of a listed todo line.
const FIELD_GAP: &str = "  ";

/// Runs a `muster todo` action against `store` for `project`, returning the rows
/// to print. Absent a subcommand, lists the project's todos.
///
/// # Errors
/// Returns [`CliError`] when a title or id is invalid, a status targets an
/// unknown todo, or the store cannot be read or written.
pub fn todo(
    command: Option<TodoCommand>,
    store: &dyn CoordinationStore,
    project: &Path,
    author: &Author,
) -> Result<Vec<Row>, CliError> {
    match command {
        None => list(store, project),
        Some(TodoCommand::Add { title, deps }) => add(store, project, author, &title, &deps),
        Some(TodoCommand::Status { id, status }) => set_status(store, project, author, &id, status),
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

/// Lists every todo for `project` as `id  status  title`, one per line.
///
/// # Errors
/// Returns [`CliError`] when the store cannot be read.
fn list(store: &dyn CoordinationStore, project: &Path) -> Result<Vec<Row>, CliError> {
    let todos = store.todos(project)?;
    if todos.is_empty() {
        return Ok(vec![Row::unlabeled(RowKind::Hint, EMPTY_NOTE)]);
    }
    Ok(todos
        .iter()
        .map(|todo| {
            Row::unlabeled(
                RowKind::Plain,
                format!(
                    "{id}{FIELD_GAP}{status}{FIELD_GAP}{title}{attribution}",
                    id = todo.id().as_ref(),
                    status = todo.status(),
                    title = todo.title().as_ref(),
                    attribution = attribution(todo.author()),
                ),
            )
        })
        .collect())
}

/// Adds a new pending todo with `title` and `deps`, printing its generated id.
///
/// # Errors
/// Returns [`CliError::InvalidTodoTitle`] for a blank title,
/// [`CliError::InvalidTodoId`] for a blank dependency id, or a store failure.
fn add(
    store: &dyn CoordinationStore,
    project: &Path,
    author: &Author,
    title: &str,
    deps: &[String],
) -> Result<Vec<Row>, CliError> {
    let title =
        TodoTitle::try_new(title).map_err(|_| CliError::InvalidTodoTitle(title.to_string()))?;
    let deps = deps
        .iter()
        .map(|dep| parse_id(dep))
        .collect::<Result<Vec<_>, _>>()?;
    let created = store.add_todo(project, author, &title, &deps)?;
    Ok(vec![Row::unlabeled(
        RowKind::Ok,
        created.id().as_ref().to_string(),
    )])
}

/// Sets the status of the todo identified by `id`.
///
/// # Errors
/// Returns [`CliError::InvalidTodoId`] for a blank id, [`CliError::UnknownTodo`]
/// when no todo has that id, or a store failure.
fn set_status(
    store: &dyn CoordinationStore,
    project: &Path,
    author: &Author,
    id: &str,
    status: TodoStatus,
) -> Result<Vec<Row>, CliError> {
    let parsed = parse_id(id)?;
    if store.set_todo_status(project, author, &parsed, status)? {
        Ok(vec![Row::unlabeled(
            RowKind::Ok,
            format!("{parsed} is {status}"),
        )])
    } else {
        Err(CliError::UnknownTodo(id.to_string()))
    }
}

/// Deletes the todo identified by `id`. Deleting an absent todo is not an error.
///
/// # Errors
/// Returns [`CliError::InvalidTodoId`] for a blank id, or a store failure.
fn remove(store: &dyn CoordinationStore, project: &Path, id: &str) -> Result<Vec<Row>, CliError> {
    let parsed = parse_id(id)?;
    if store.delete_todo(project, &parsed)? {
        Ok(vec![Row::unlabeled(
            RowKind::Ok,
            format!("removed '{parsed}'"),
        )])
    } else {
        Ok(vec![Row::unlabeled(
            RowKind::Hint,
            format!("no todo '{parsed}' to remove"),
        )])
    }
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, path::PathBuf};

    use super::*;
    use crate::domain::{
        config::ConfigError,
        coordination::{KeyValue, KvKey, Scratchpad, ScratchpadKey, Todo},
    };

    /// An in-memory coordination store recording todos for one project.
    #[derive(Default)]
    struct FakeStore {
        todos: RefCell<Vec<Todo>>,
    }

    impl CoordinationStore for FakeStore {
        fn todos(&self, _project: &Path) -> Result<Vec<Todo>, ConfigError> {
            Ok(self.todos.borrow().clone())
        }

        fn todo(&self, _project: &Path, id: &TodoId) -> Result<Option<Todo>, ConfigError> {
            Ok(self
                .todos
                .borrow()
                .iter()
                .find(|todo| todo.id() == id)
                .cloned())
        }

        fn add_todo(
            &self,
            _project: &Path,
            author: &Author,
            title: &TodoTitle,
            deps: &[TodoId],
        ) -> Result<Todo, ConfigError> {
            let todo = Todo::builder()
                .id(TodoId::generate().unwrap())
                .title(title.clone())
                .status(TodoStatus::Pending)
                .deps(deps.to_vec())
                .author(author.clone())
                .updated_at(0)
                .build();
            self.todos.borrow_mut().push(todo.clone());
            Ok(todo)
        }

        fn set_todo_status(
            &self,
            _project: &Path,
            author: &Author,
            id: &TodoId,
            status: TodoStatus,
        ) -> Result<bool, ConfigError> {
            let mut todos = self.todos.borrow_mut();
            match todos.iter_mut().find(|todo| todo.id() == id) {
                Some(todo) => {
                    *todo = Todo::builder()
                        .id(todo.id().clone())
                        .title(todo.title().clone())
                        .status(status)
                        .deps(todo.deps().clone())
                        .author(author.clone())
                        .updated_at(0)
                        .build();
                    Ok(true)
                },
                None => Ok(false),
            }
        }

        fn delete_todo(&self, _project: &Path, id: &TodoId) -> Result<bool, ConfigError> {
            let mut todos = self.todos.borrow_mut();
            let before = todos.len();
            todos.retain(|todo| todo.id() != id);
            Ok(todos.len() != before)
        }

        fn scratchpads(&self, _project: &Path) -> Result<Vec<Scratchpad>, ConfigError> {
            unreachable!("todo commands never touch scratchpads")
        }

        fn scratchpad(
            &self,
            _project: &Path,
            _key: &ScratchpadKey,
        ) -> Result<Option<Scratchpad>, ConfigError> {
            unreachable!("todo commands never touch scratchpads")
        }

        fn set_scratchpad(
            &self,
            _project: &Path,
            _author: &Author,
            _key: &ScratchpadKey,
            _body: &str,
        ) -> Result<(), ConfigError> {
            unreachable!("todo commands never touch scratchpads")
        }

        fn delete_scratchpad(
            &self,
            _project: &Path,
            _key: &ScratchpadKey,
        ) -> Result<bool, ConfigError> {
            unreachable!("todo commands never touch scratchpads")
        }

        fn values(&self, _project: &Path) -> Result<Vec<KeyValue>, ConfigError> {
            unreachable!("todo commands never touch key-values")
        }

        fn value(&self, _project: &Path, _key: &KvKey) -> Result<Option<KeyValue>, ConfigError> {
            unreachable!("todo commands never touch key-values")
        }

        fn set_value(
            &self,
            _project: &Path,
            _author: &Author,
            _key: &KvKey,
            _value: &str,
        ) -> Result<(), ConfigError> {
            unreachable!("todo commands never touch key-values")
        }

        fn delete_value(&self, _project: &Path, _key: &KvKey) -> Result<bool, ConfigError> {
            unreachable!("todo commands never touch key-values")
        }

        fn import(
            &self,
            _project: &Path,
            _scratchpads: &[Scratchpad],
            _todos: &[Todo],
            _values: &[KeyValue],
        ) -> Result<(), ConfigError> {
            unreachable!("todo commands never import")
        }
    }

    fn project() -> PathBuf {
        PathBuf::from("/repo/muster.yml")
    }

    /// The added id is the row detail, so a script can capture it directly.
    fn added_id(store: &FakeStore, title: &str) -> String {
        add(store, &project(), &Author::human(), title, &[]).unwrap()[0]
            .detail()
            .clone()
    }

    /// Adding prints the id; listing shows it with the pending status and title.
    #[test]
    fn add_then_list() {
        let store = FakeStore::default();
        let id = added_id(&store, "wire the store");

        let listed = list(&store, &project()).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].detail().starts_with(&id));
        assert!(listed[0].detail().contains("pending"));
        assert!(listed[0].detail().contains("wire the store"));
    }

    /// Setting a status updates the listed line; an unknown id errors.
    #[test]
    fn status_updates_and_unknown_errors() {
        let store = FakeStore::default();
        let id = added_id(&store, "task");

        let ok = set_status(&store, &project(), &Author::human(), &id, TodoStatus::Done).unwrap();
        assert_eq!(ok[0].kind(), RowKind::Ok);
        assert!(
            list(&store, &project()).unwrap()[0]
                .detail()
                .contains("done")
        );

        let unknown = TodoId::generate().unwrap();
        assert!(matches!(
            set_status(
                &store,
                &project(),
                &Author::human(),
                unknown.as_ref(),
                TodoStatus::Done
            ),
            Err(CliError::UnknownTodo(_))
        ));
    }

    /// Removing succeeds once, then reports absence idempotently.
    #[test]
    fn remove_is_idempotent() {
        let store = FakeStore::default();
        let id = added_id(&store, "task");

        assert_eq!(
            remove(&store, &project(), &id).unwrap()[0].kind(),
            RowKind::Ok
        );
        assert_eq!(
            remove(&store, &project(), &id).unwrap()[0].kind(),
            RowKind::Hint
        );
    }

    /// A blank id or title is rejected before the store is touched.
    #[test]
    fn blank_inputs_are_rejected() {
        let store = FakeStore::default();
        assert!(matches!(
            add(&store, &project(), &Author::human(), "   ", &[]),
            Err(CliError::InvalidTodoTitle(_))
        ));
        assert!(matches!(
            add(&store, &project(), &Author::human(), "task", &[
                "  ".to_string()
            ]),
            Err(CliError::InvalidTodoId(_))
        ));
        assert!(matches!(
            remove(&store, &project(), "  "),
            Err(CliError::InvalidTodoId(_))
        ));
    }

    /// An empty store lists a hint rather than nothing.
    #[test]
    fn list_reports_empty() {
        let store = FakeStore::default();
        assert_eq!(list(&store, &project()).unwrap()[0].detail(), EMPTY_NOTE);
    }
}
