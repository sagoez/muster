//! The coordination tools: shared todos with claims, short named locks,
//! notes and key-values, written as this connection's agent.

use std::time::{Duration, Instant};

use rmcp::{
    RoleServer, handler::server::wrapper::Parameters, service::RequestContext, tool, tool_router,
};
use serde_json::json as object_of;
use tokio::time;

use super::{
    bounds::{CALL_BOUND, page_size},
    key_arg::KeyArg,
    kv_write_arg::KvWriteArg,
    lock_acquire_arg::LockAcquireArg,
    lock_name_arg::LockNameArg,
    note_entry::NoteEntry,
    note_write_arg::NoteWriteArg,
    process_fields::invalid,
    progress::with_progress,
    read_arg::ReadArg,
    server::{MusterMcp, json, structured, unix_now},
    todo_add_arg::TodoAddArg,
    todo_claim_arg::TodoClaimArg,
    todo_id_arg::TodoIdArg,
    todo_list_arg::TodoListArg,
    todo_update_arg::TodoUpdateArg,
    todo_view::TodoView,
    tool_error::ToolError,
    tool_result::ToolResult,
};
use crate::{
    adapter::wire::WireForm,
    domain::{
        coordination::{
            Claim, KvKey, LOCK_TTL_MAX, LockAttempt, LockName, LockNote, LockRelease, LockTtl,
            ScratchpadKey, TODO_LEASE_MAX, TodoChange, TodoId, TodoLease, TodoStatus, TodoTitle,
            TodoUpdate, claimable,
        },
        port::CoordinationStore,
        value::ProjectRoot,
    },
};

/// Pause between attempts at a lock someone else holds.
const LOCK_RETRY: Duration = Duration::from_millis(250);
/// What `owner` takes to mean the calling agent.
const OWNER_ME: &str = "me";
/// The fields the coordination tools name in a refusal.
const STATUS_FIELD: &str = "status";
const NAME_FIELD: &str = "name";
const NOTE_FIELD: &str = "note";
const LEASE_FIELD: &str = "lease_s";
const TTL_FIELD: &str = "ttl_s";
const RELEASE_FIELD: &str = "release";
/// The statuses a todo takes.
const STATUS_FORM: &str = "pending, in-progress or done";
/// The form a lock name is written in.
const LOCK_NAME_FORM: &str = "1 to 64 of a-z, 0-9, '.', '_' and '-'";
/// The form a lock note is written in.
const LOCK_NOTE_FORM: &str = "one line of 1 to 200 characters";
/// The form a lease or a TTL is written in, followed by its maximum.
const SECONDS_FORM: &str = "whole seconds from 1 to ";
/// What `release` cannot be combined with.
const RELEASE_FORM: &str = "release only without a status other than pending";
/// The tool `todo_update`'s refusal names.
const TODO_UPDATE: &str = "todo_update";
/// Keys of the results built here.
const TODOS: &str = "todos";
const TODO: &str = "todo";
const NOTES: &str = "notes";
const ENTRIES: &str = "entries";
const SAVED: &str = "saved";
const DELETED: &str = "deleted";
const KEY: &str = "key";
const ID: &str = "id";
const CLAIMED: &str = "claimed";
const ACQUIRED: &str = "acquired";
const RELEASED: &str = "released";
const OWNER: &str = "owner";
const NOTE: &str = "note";
const EXPIRES_IN_S: &str = "expires_in_s";

/// The scratchpad key an agent typed as `key`.
///
/// # Errors
/// Returns the caller's mistake for a blank key.
fn note_key(key: &str) -> Result<ScratchpadKey, ToolError> {
    ScratchpadKey::try_new(key.to_string()).map_err(|_| ToolError::InvalidNoteKey(key.to_string()))
}

/// The todo id an agent typed as `id`.
///
/// # Errors
/// Returns the caller's mistake for a blank id.
fn todo_id(id: &str) -> Result<TodoId, ToolError> {
    TodoId::try_new(id.to_string()).map_err(|_| ToolError::InvalidTodoId(id.to_string()))
}

/// The key-value key an agent typed as `key`.
///
/// # Errors
/// Returns the caller's mistake for a blank key.
fn kv_key(key: &str) -> Result<KvKey, ToolError> {
    KvKey::try_new(key.to_string()).map_err(|_| ToolError::InvalidKvKey(key.to_string()))
}

/// The lock name an agent typed as `name`.
///
/// # Errors
/// Returns the caller's mistake for a name out of form.
fn lock_name(name: &str) -> Result<LockName, ToolError> {
    LockName::try_new(name).map_err(|_| invalid(NAME_FIELD, LOCK_NAME_FORM, name))
}

/// The todo status an agent typed as `raw`.
///
/// # Errors
/// Returns the caller's mistake for a status out of form.
fn todo_status(raw: &str) -> Result<TodoStatus, ToolError> {
    raw.trim()
        .parse()
        .map_err(|_| invalid(STATUS_FIELD, STATUS_FORM, raw))
}

/// The lock note an agent typed as `note`.
///
/// # Errors
/// Returns the caller's mistake for a blank, multi-line or overlong note.
fn lock_note(note: &str) -> Result<LockNote, ToolError> {
    LockNote::try_new(note).map_err(|_| invalid(NOTE_FIELD, LOCK_NOTE_FORM, note))
}

/// The claim lease `seconds` asks for, the default when absent.
///
/// # Errors
/// Returns the caller's mistake for a lease out of range.
fn todo_lease(seconds: Option<u64>) -> Result<TodoLease, ToolError> {
    seconds.map_or(Ok(TodoLease::DEFAULT), |seconds| {
        TodoLease::try_new(Duration::from_secs(seconds)).map_err(|_| {
            invalid(
                LEASE_FIELD,
                &format!("{SECONDS_FORM}{}", TODO_LEASE_MAX.as_secs()),
                &seconds.to_string(),
            )
        })
    })
}

/// The lock time-to-live `seconds` asks for, the default when absent.
///
/// # Errors
/// Returns the caller's mistake for a TTL out of range.
fn lock_ttl(seconds: Option<u64>) -> Result<LockTtl, ToolError> {
    seconds.map_or(Ok(LockTtl::DEFAULT), |seconds| {
        LockTtl::try_new(Duration::from_secs(seconds)).map_err(|_| {
            invalid(
                TTL_FIELD,
                &format!("{SECONDS_FORM}{}", LOCK_TTL_MAX.as_secs()),
                &seconds.to_string(),
            )
        })
    })
}

/// The refusal of the unknown todo `id`, listing the project's ids.
fn unknown_todo(store: &dyn CoordinationStore, root: &ProjectRoot, id: &TodoId) -> ToolError {
    ToolError::UnknownTodo {
        id: id.to_string(),
        valid: store
            .todos(root)
            .map(|todos| todos.iter().map(|todo| todo.id().to_string()).collect())
            .unwrap_or_default(),
    }
}

#[tool_router(router = coordination_router, vis = "pub(super)")]
impl MusterMcp {
    /// Lists a project's todos.
    #[tool(
        description = "Shared todos of this project, oldest first. status keeps one status \
            (pending, in-progress or done); owner keeps those held by \"me\" or by another \
            agent's label; claimable=true lists only pending todos whose dependencies are done. \
            Returns {todos: [{id, title, status, deps, owner, expires_in_s, stale_owner, author, \
            updated_at}]}; an expired claim reads as pending with its holder as stale_owner."
    )]
    pub(super) async fn todo_list(&self, Parameters(arg): Parameters<TodoListArg>) -> ToolResult {
        let status = arg.status().as_deref().map(todo_status).transpose()?;
        let me = self.identity.author().clone();
        let owner = arg.owner().clone().map(|owner| {
            if owner.trim() == OWNER_ME {
                me.label().to_string()
            } else {
                owner.trim().to_string()
            }
        });
        let only_claimable = arg.claimable().unwrap_or(false);
        let limit = page_size(arg.limit());
        let todos = self
            .blocking(move |deps| {
                let project = deps.project(arg.project().as_deref())?;
                Ok(deps.coordination().todos(project.root())?)
            })
            .await?;
        let now = unix_now();
        let listed: Vec<_> = if only_claimable {
            claimable(&todos, now).collect()
        } else {
            todos.iter().collect()
        };
        let page: Vec<TodoView> = listed
            .into_iter()
            .map(|todo| TodoView::at(todo, now))
            .filter(|view| status.is_none_or(|status| *view.status() == status))
            .filter(|view| {
                owner.as_deref().is_none_or(|owner| {
                    view.owner()
                        .as_ref()
                        .is_some_and(|held| held.label() == owner)
                })
            })
            .take(limit)
            .collect();
        structured(object_of!({ TODOS: json(&page)? }), false)
    }

    /// Adds a todo.
    #[tool(
        description = "Add a pending todo. Takes title and deps, the ids of todos it depends \
            on. Returns the todo {id, title, status, deps, author, updated_at}, with you (the \
            label your writes carry) the first time."
    )]
    pub(super) async fn todo_add(&self, Parameters(arg): Parameters<TodoAddArg>) -> ToolResult {
        let title = TodoTitle::try_new(arg.title().clone())
            .map_err(|_| ToolError::InvalidTodoTitle(arg.title().clone()))?;
        let deps = arg
            .deps()
            .iter()
            .flatten()
            .map(|dep| todo_id(dep))
            .collect::<Result<Vec<_>, _>>()?;
        let me = self.identity.author().clone();
        let created = self
            .blocking(move |store| {
                let project = store.project(arg.project().as_deref())?;
                Ok(store
                    .coordination()
                    .add_todo(project.root(), &me, &title, &deps)?)
            })
            .await?;
        structured(
            self.introduced(json(&TodoView::at(&created, unix_now()))?),
            false,
        )
    }

    /// Claims a todo.
    #[tool(
        description = "Take a todo so no other agent works on it. Without id, takes the oldest \
            claimable todo. Claiming one you hold renews its lease (lease_s, default \
            <default-lease-s>, at most <max-lease-s>). Returns {claimed: true, todo}, or \
            {claimed: false, owner, expires_in_s} when another agent holds it, or {claimed: \
            false} when nothing is claimable."
    )]
    pub(super) async fn todo_claim(&self, Parameters(arg): Parameters<TodoClaimArg>) -> ToolResult {
        let id = arg.id().as_deref().map(todo_id).transpose()?;
        let lease = todo_lease(arg.lease_s())?;
        let me = self.identity.author().clone();
        let claim = self
            .blocking(move |deps| {
                let project = deps.project(arg.project().as_deref())?;
                let store = deps.coordination().as_ref();
                match store.claim_todo(project.root(), &me, id.as_ref(), lease)? {
                    Claim::Missing => match &id {
                        Some(id) => Err(unknown_todo(store, project.root(), id)),
                        None => Ok(Claim::NothingClaimable),
                    },
                    claim => Ok(claim),
                }
            })
            .await?;
        let now = unix_now();
        let value = match claim {
            Claim::Claimed(todo) => {
                object_of!({ CLAIMED: true, TODO: json(&TodoView::at(&todo, now))? })
            },
            Claim::HeldBy { owner, expires_in } => {
                object_of!({ CLAIMED: false, OWNER: owner.to_string(), EXPIRES_IN_S: expires_in.as_secs() })
            },
            Claim::Unavailable(todo) => {
                object_of!({ CLAIMED: false, TODO: json(&TodoView::at(&todo, now))? })
            },
            Claim::NothingClaimable | Claim::Missing => object_of!({ CLAIMED: false }),
        };
        structured(self.introduced(value), false)
    }

    /// Changes a todo.
    #[tool(
        description = "Change a todo: status (pending, in-progress or done; done finishes it \
            and clears its owner), title, or release=true to hand it back as pending. A todo \
            another agent holds is refused unless force=true. Returns the todo."
    )]
    pub(super) async fn todo_update(
        &self,
        Parameters(arg): Parameters<TodoUpdateArg>,
    ) -> ToolResult {
        let id = todo_id(arg.id())?;
        let status = arg.status().as_deref().map(todo_status).transpose()?;
        let title = arg
            .title()
            .as_ref()
            .map(|title| {
                TodoTitle::try_new(title.clone())
                    .map_err(|_| ToolError::InvalidTodoTitle(title.clone()))
            })
            .transpose()?;
        let status = match (arg.release().unwrap_or(false), status) {
            (false, status) => status,
            (true, None | Some(TodoStatus::Pending)) => Some(TodoStatus::Pending),
            (true, Some(other)) => {
                return Err(invalid(RELEASE_FIELD, RELEASE_FORM, &other.to_string()).into());
            },
        };
        if status.is_none() && title.is_none() {
            return Err(ToolError::NothingToChange(TODO_UPDATE).into());
        }
        let change = TodoChange::builder()
            .status(status)
            .title(title)
            .force(arg.force().unwrap_or(false))
            .build();
        let me = self.identity.author().clone();
        let updated = self
            .blocking(move |deps| {
                let project = deps.project(arg.project().as_deref())?;
                let store = deps.coordination().as_ref();
                match store.update_todo(project.root(), &me, &id, &change)? {
                    TodoUpdate::Updated(todo) => Ok(todo),
                    TodoUpdate::HeldBy { owner, expires_in } => Err(ToolError::HeldByOther {
                        id: id.to_string(),
                        owner: owner.label().to_string(),
                        expires_in_s: expires_in.as_secs(),
                    }),
                    TodoUpdate::Missing => Err(unknown_todo(store, project.root(), &id)),
                }
            })
            .await?;
        structured(
            self.introduced(json(&TodoView::at(&updated, unix_now()))?),
            false,
        )
    }

    /// Deletes a todo.
    #[tool(
        description = "Delete a todo. Returns {id, deleted}; deleted is false when there was none."
    )]
    pub(super) async fn todo_delete(&self, Parameters(arg): Parameters<TodoIdArg>) -> ToolResult {
        let id = todo_id(arg.id())?;
        let deleted = {
            let id = id.clone();
            self.blocking(move |deps| {
                let project = deps.project(arg.project().as_deref())?;
                Ok(deps.coordination().delete_todo(project.root(), &id)?)
            })
            .await?
        };
        structured(object_of!({ ID: id, DELETED: deleted }), false)
    }

    /// Takes a named lock.
    #[tool(
        description = "Take a short named lock, such as \"migrations\" or \"db-reset\", before \
            work that two agents must not do at once. Expires after ttl_s (default \
            <default-ttl-s>, at most <max-ttl-s>); acquiring it again renews it. With wait_s (at \
            most <call-bound-s>), waits for it to free up. Returns {acquired: true, \
            expires_in_s}, or {acquired: false, owner, note, expires_in_s}."
    )]
    pub(super) async fn lock_acquire(
        &self,
        Parameters(arg): Parameters<LockAcquireArg>,
        context: RequestContext<RoleServer>,
    ) -> ToolResult {
        with_progress(&context, self.acquire(arg)).await
    }

    /// Releases a named lock.
    #[tool(
        description = "Release a lock you hold. Returns {released: true}, or {released: false, \
            owner} when someone else holds it, or {released: false} when nobody does."
    )]
    pub(super) async fn lock_release(
        &self,
        Parameters(arg): Parameters<LockNameArg>,
    ) -> ToolResult {
        let name = lock_name(arg.name())?;
        let me = self.identity.author().clone();
        let release = self
            .blocking(move |deps| {
                let project = deps.project(arg.project().as_deref())?;
                Ok(deps
                    .coordination()
                    .release_lock(project.root(), &me, &name)?)
            })
            .await?;
        structured(
            match release {
                LockRelease::Released => object_of!({ RELEASED: true }),
                LockRelease::NotHeld => object_of!({ RELEASED: false }),
                LockRelease::HeldBy { owner } => {
                    object_of!({ RELEASED: false, OWNER: owner.to_string() })
                },
            },
            false,
        )
    }

    /// Reads one note, or lists them.
    #[tool(
        description = "Without key, lists the shared notes {notes: [{key, author, updated_at}]} \
            without bodies (limit, default <default-list-limit>, at most <max-list-limit>). \
            With key, returns {key, body, author, updated_at}; an unknown key is flagged \
            isError."
    )]
    pub(super) async fn note_read(&self, Parameters(arg): Parameters<ReadArg>) -> ToolResult {
        let key = arg.key().as_deref().map(note_key).transpose()?;
        let limit = page_size(arg.limit());
        let notes = self
            .blocking(move |deps| {
                let project = deps.project(arg.project().as_deref())?;
                Ok(deps.coordination().scratchpads(project.root())?)
            })
            .await?;
        let Some(key) = key else {
            let page: Vec<NoteEntry> = notes.iter().take(limit).map(NoteEntry::from).collect();
            return structured(object_of!({ NOTES: json(&page)? }), false);
        };
        match notes.iter().find(|note| note.key() == &key) {
            Some(note) => structured(json(&note.to_wire())?, false),
            None => Err(ToolError::UnknownNote {
                key: key.to_string(),
                valid: notes.iter().map(|note| note.key().to_string()).collect(),
            }
            .into()),
        }
    }

    /// Writes a note.
    #[tool(
        description = "Create or replace a shared markdown note that other agents and the human \
            read. Returns {saved: key}, with you (the label your writes carry) the first time."
    )]
    pub(super) async fn note_write(&self, Parameters(arg): Parameters<NoteWriteArg>) -> ToolResult {
        let key = note_key(arg.key())?;
        let me = self.identity.author().clone();
        let saved = key.to_string();
        self.blocking(move |deps| {
            let project = deps.project(arg.project().as_deref())?;
            Ok(deps
                .coordination()
                .set_scratchpad(project.root(), &me, &key, arg.body())?)
        })
        .await?;
        structured(self.introduced(object_of!({ SAVED: saved })), false)
    }

    /// Deletes a note.
    #[tool(
        description = "Delete a note. Returns {key, deleted}; deleted is false when there was none."
    )]
    pub(super) async fn note_delete(&self, Parameters(arg): Parameters<KeyArg>) -> ToolResult {
        let key = note_key(arg.key())?;
        let named = key.to_string();
        let deleted = self
            .blocking(move |deps| {
                let project = deps.project(arg.project().as_deref())?;
                Ok(deps
                    .coordination()
                    .delete_scratchpad(project.root(), &key)?)
            })
            .await?;
        structured(object_of!({ KEY: named, DELETED: deleted }), false)
    }

    /// Reads one key-value entry, or lists them.
    #[tool(
        description = "Without key, lists the shared key-values {entries: [{key, value, author, \
            updated_at}]} (limit, default <default-list-limit>, at most <max-list-limit>). With \
            key, returns that entry; an unknown key is flagged isError."
    )]
    pub(super) async fn kv_read(&self, Parameters(arg): Parameters<ReadArg>) -> ToolResult {
        let key = arg.key().as_deref().map(kv_key).transpose()?;
        let limit = page_size(arg.limit());
        let entries = self
            .blocking(move |deps| {
                let project = deps.project(arg.project().as_deref())?;
                Ok(deps.coordination().values(project.root())?)
            })
            .await?;
        let Some(key) = key else {
            let page: Vec<_> = entries.iter().take(limit).map(WireForm::to_wire).collect();
            return structured(object_of!({ ENTRIES: json(&page)? }), false);
        };
        match entries.iter().find(|entry| entry.key() == &key) {
            Some(entry) => structured(json(&entry.to_wire())?, false),
            None => Err(ToolError::UnknownValue {
                key: key.to_string(),
                valid: entries
                    .iter()
                    .map(|entry| entry.key().to_string())
                    .collect(),
            }
            .into()),
        }
    }

    /// Writes a key-value entry.
    #[tool(
        description = "Create or replace a small shared fact, such as a chosen port or a build \
            id. Returns {saved: key}, with you (the label your writes carry) the first time."
    )]
    pub(super) async fn kv_write(&self, Parameters(arg): Parameters<KvWriteArg>) -> ToolResult {
        let key = kv_key(arg.key())?;
        let me = self.identity.author().clone();
        let saved = key.to_string();
        self.blocking(move |deps| {
            let project = deps.project(arg.project().as_deref())?;
            Ok(deps
                .coordination()
                .set_value(project.root(), &me, &key, arg.value())?)
        })
        .await?;
        structured(self.introduced(object_of!({ SAVED: saved })), false)
    }

    /// Deletes a key-value entry.
    #[tool(
        description = "Delete a key-value. Returns {key, deleted}; deleted is false when there was none."
    )]
    pub(super) async fn kv_delete(&self, Parameters(arg): Parameters<KeyArg>) -> ToolResult {
        let key = kv_key(arg.key())?;
        let named = key.to_string();
        let deleted = self
            .blocking(move |deps| {
                let project = deps.project(arg.project().as_deref())?;
                Ok(deps.coordination().delete_value(project.root(), &key)?)
            })
            .await?;
        structured(object_of!({ KEY: named, DELETED: deleted }), false)
    }
}

impl MusterMcp {
    /// What `lock_acquire` does, without its progress notifications:
    /// attempts the lock, and while someone else holds it and `wait_s` has
    /// not passed, tries again every [`LOCK_RETRY`].
    ///
    /// # Errors
    /// Returns the caller's mistake for a bad name, or a fault of muster's.
    pub(super) async fn acquire(&self, arg: LockAcquireArg) -> ToolResult {
        let name = lock_name(arg.name())?;
        let ttl = lock_ttl(arg.ttl_s())?;
        let note = arg.note().as_deref().map(lock_note).transpose()?;
        let deadline = Instant::now()
            + arg
                .wait_s()
                .map_or(Duration::ZERO, Duration::from_secs)
                .min(CALL_BOUND);
        let me = self.identity.author().clone();
        let project = arg.project().clone();
        loop {
            let (name, me, project, note) =
                (name.clone(), me.clone(), project.clone(), note.clone());
            let attempt = self
                .blocking(move |deps| {
                    let project = deps.project(project.as_deref())?;
                    Ok(deps.coordination().acquire_lock(
                        project.root(),
                        &me,
                        &name,
                        ttl,
                        note.as_ref(),
                    )?)
                })
                .await?;
            let held = match attempt {
                LockAttempt::Acquired { expires_in } => {
                    let value = object_of!({ ACQUIRED: true, EXPIRES_IN_S: expires_in.as_secs() });
                    return structured(self.introduced(value), false);
                },
                LockAttempt::HeldBy {
                    owner,
                    note,
                    expires_in,
                } => object_of!({
                    ACQUIRED: false,
                    OWNER: owner.to_string(),
                    NOTE: note.as_ref().map(ToString::to_string),
                    EXPIRES_IN_S: expires_in.as_secs(),
                }),
            };
            if Instant::now() + LOCK_RETRY > deadline {
                return structured(held, false);
            }
            time::sleep(LOCK_RETRY).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::mcp::test_support::{
        AGENT, content, fixture, refusal, register, result_text, runtime, server_as,
    };

    /// A note written through the tool reads back by key and in the index,
    /// which leaves out bodies; the first write says who the agent is.
    #[test]
    fn a_note_round_trips_and_the_index_omits_bodies() {
        let fixture = fixture();
        register(&fixture);
        let server = &fixture.server;

        runtime().block_on(async {
            let saved = content(
                server
                    .note_write(Parameters(
                        NoteWriteArg::builder()
                            .key("plan".to_string())
                            .body("ship it".to_string())
                            .build(),
                    ))
                    .await,
            );
            assert_eq!(saved[SAVED], "plan");
            assert_eq!(saved["you"], AGENT);

            let listed = content(server.note_read(Parameters(ReadArg::default())).await);
            assert_eq!(listed[NOTES][0][KEY], "plan");
            assert_eq!(listed[NOTES][0]["author"], AGENT);
            assert!(listed[NOTES][0].get("body").is_none());

            let read = content(
                server
                    .note_read(Parameters(
                        ReadArg::builder().key(Some("plan".to_string())).build(),
                    ))
                    .await,
            );
            assert_eq!(read["body"], "ship it");

            let unknown = refusal(
                server
                    .note_read(Parameters(
                        ReadArg::builder().key(Some("plna".to_string())).build(),
                    ))
                    .await,
            );
            assert!(unknown.contains("note_read without key"), "{unknown}");
        });
    }

    /// An unknown key is flagged with its kind and the keys there are.
    #[test]
    fn an_unknown_key_lists_the_keys() {
        let fixture = fixture();
        register(&fixture);

        runtime().block_on(async {
            content(
                fixture
                    .server
                    .kv_write(Parameters(
                        KvWriteArg::builder()
                            .key("port".to_string())
                            .value("3000".to_string())
                            .build(),
                    ))
                    .await,
            );
            let result = fixture
                .server
                .kv_read(Parameters(
                    ReadArg::builder().key(Some("prot".to_string())).build(),
                ))
                .await
                .unwrap_err()
                .answer()
                .unwrap();
            assert_eq!(result.is_error, Some(true));
            let error = &result.structured_content.clone().expect("structured")["error"];
            assert_eq!(error["kind"], "unknown-key");
            assert_eq!(error["candidates"], serde_json::json!(["port"]));
            assert!(result_text(&result).contains("kv_read"));
        });
    }

    /// Coordination outside every project names process_add.
    #[test]
    fn coordination_outside_a_project_names_process_add() {
        let fixture = fixture();
        runtime().block_on(async {
            let text = refusal(
                fixture
                    .server
                    .note_write(Parameters(
                        NoteWriteArg::builder()
                            .key("plan".to_string())
                            .body("x".to_string())
                            .build(),
                    ))
                    .await,
            );
            assert!(text.contains("process_add"), "{text}");
        });
    }

    /// claimable lists only pending todos whose dependencies are done, and
    /// a status filter keeps one status.
    #[test]
    fn todo_list_filters_claimable_and_status() {
        let fixture = fixture();
        register(&fixture);
        let server = &fixture.server;

        runtime().block_on(async {
            let first = content(
                server
                    .todo_add(Parameters(
                        TodoAddArg::builder().title("schema".to_string()).build(),
                    ))
                    .await,
            );
            let first_id = first[ID].as_str().unwrap().to_string();
            content(
                server
                    .todo_add(Parameters(
                        TodoAddArg::builder()
                            .title("api".to_string())
                            .deps(Some(vec![first_id.clone()]))
                            .build(),
                    ))
                    .await,
            );
            let claimable = content(
                server
                    .todo_list(Parameters(
                        TodoListArg::builder().claimable(Some(true)).build(),
                    ))
                    .await,
            );
            let titles: Vec<&str> = claimable[TODOS]
                .as_array()
                .unwrap()
                .iter()
                .map(|todo| todo["title"].as_str().unwrap())
                .collect();
            assert_eq!(titles, vec!["schema"]);
            let done = content(
                server
                    .todo_list(Parameters(
                        TodoListArg::builder()
                            .status(Some("done".to_string()))
                            .build(),
                    ))
                    .await,
            );
            assert!(done[TODOS].as_array().unwrap().is_empty());
            let bad = refusal(
                server
                    .todo_list(Parameters(
                        TodoListArg::builder()
                            .status(Some("finished".to_string()))
                            .build(),
                    ))
                    .await,
            );
            assert!(bad.contains("in-progress"), "{bad}");
        });
    }

    /// A todo update that changes nothing, and a lock name out of form, are
    /// refused before the store is asked.
    #[test]
    fn empty_updates_and_bad_lock_names_are_refused() {
        let fixture = fixture();
        register(&fixture);
        runtime().block_on(async {
            let nothing = refusal(
                fixture
                    .server
                    .todo_update(Parameters(
                        TodoUpdateArg::builder().id("x".to_string()).build(),
                    ))
                    .await,
            );
            assert!(nothing.contains("todo_update changed nothing"), "{nothing}");
            let name = refusal(
                fixture
                    .server
                    .lock_release(Parameters(
                        LockNameArg::builder().name("DB Reset".to_string()).build(),
                    ))
                    .await,
            );
            assert!(name.contains("a-z"), "{name}");
        });
    }

    /// A lease or a TTL is the default when absent, and refused naming its
    /// field when out of range.
    #[test]
    fn leases_and_ttls_are_bounded() {
        assert_eq!(todo_lease(None).unwrap(), TodoLease::DEFAULT);
        assert_eq!(lock_ttl(None).unwrap(), LockTtl::DEFAULT);
        assert!(
            lock_ttl(Some(0))
                .unwrap_err()
                .to_string()
                .contains(TTL_FIELD)
        );
        assert!(
            todo_lease(Some(TODO_LEASE_MAX.as_secs() + 1))
                .unwrap_err()
                .to_string()
                .contains(LEASE_FIELD)
        );
    }

    /// A claim holds the todo against a second agent, who sees the owner and
    /// is refused a change unless it forces; releasing hands it back.
    #[test]
    fn a_claim_holds_the_todo_against_another_agent() {
        let fixture = fixture();
        register(&fixture);
        let other = server_as(&fixture, "other");

        runtime().block_on(async {
            let added = content(
                fixture
                    .server
                    .todo_add(Parameters(
                        TodoAddArg::builder().title("schema".to_string()).build(),
                    ))
                    .await,
            );
            let id = added[ID].as_str().unwrap().to_string();

            let claimed = content(
                fixture
                    .server
                    .todo_claim(Parameters(TodoClaimArg::builder().build()))
                    .await,
            );
            assert_eq!(claimed[CLAIMED], true);
            assert_eq!(claimed[TODO][OWNER], AGENT);
            assert_eq!(claimed[TODO]["status"], "in-progress");

            let held = content(
                other
                    .todo_claim(Parameters(
                        TodoClaimArg::builder().id(Some(id.clone())).build(),
                    ))
                    .await,
            );
            assert_eq!(held[CLAIMED], false);
            assert_eq!(held[OWNER], AGENT);
            assert!(held[EXPIRES_IN_S].as_u64().unwrap() > 0);

            let refused = refusal(
                other
                    .todo_update(Parameters(
                        TodoUpdateArg::builder()
                            .id(id.clone())
                            .status(Some("done".to_string()))
                            .build(),
                    ))
                    .await,
            );
            assert!(
                refused.contains("held by") && refused.contains(AGENT),
                "{refused}"
            );

            let released = content(
                fixture
                    .server
                    .todo_update(Parameters(
                        TodoUpdateArg::builder()
                            .id(id.clone())
                            .release(Some(true))
                            .build(),
                    ))
                    .await,
            );
            assert_eq!(released["status"], "pending");
            assert!(released.get(OWNER).is_none());

            let unknown = refusal(
                other
                    .todo_claim(Parameters(
                        TodoClaimArg::builder().id(Some("nope".to_string())).build(),
                    ))
                    .await,
            );
            assert!(unknown.contains("todo_list"), "{unknown}");
        });
    }

    /// A lock is exclusive between agents, names its holder and note to the
    /// other, and frees on release; a second agent cannot release it.
    #[test]
    fn a_lock_is_exclusive_between_agents() {
        let fixture = fixture();
        register(&fixture);
        let other = server_as(&fixture, "other");
        let acquire = |name: &str, note: Option<&str>| {
            LockAcquireArg::builder()
                .name(name.to_string())
                .note(note.map(str::to_string))
                .build()
        };
        let release =
            |name: &str| Parameters(LockNameArg::builder().name(name.to_string()).build());

        runtime().block_on(async {
            let taken = content(
                fixture
                    .server
                    .acquire(acquire("migrations", Some("adding users")))
                    .await,
            );
            assert_eq!(taken[ACQUIRED], true);

            let held = content(other.acquire(acquire("migrations", None)).await);
            assert_eq!(held[ACQUIRED], false);
            assert_eq!(held[OWNER], AGENT);
            assert_eq!(held[NOTE], "adding users");

            let not_theirs = content(other.lock_release(release("migrations")).await);
            assert_eq!(not_theirs[RELEASED], false);
            assert_eq!(not_theirs[OWNER], AGENT);

            let freed = content(fixture.server.lock_release(release("migrations")).await);
            assert_eq!(freed[RELEASED], true);
            let retaken = content(other.acquire(acquire("migrations", None)).await);
            assert_eq!(retaken[ACQUIRED], true);
        });
    }
}
