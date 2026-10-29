use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use rmcp::{
    ErrorData, ServerHandler, ServiceExt,
    handler::server::wrapper::Parameters,
    model::{CallToolResult, ContentBlock, Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
    transport::stdio,
};
use serde::Serialize;
use tokio::task;

use super::{
    key_arg::KeyArg, kv_set_arg::KvSetArg, list_arg::ListArg, mcp_error::McpError,
    name_arg::NameArg, scratchpad_entry::ScratchpadEntry, scratchpad_set_arg::ScratchpadSetArg,
    send_input_arg::SendInputArg, todo_add_arg::TodoAddArg, todo_id_arg::TodoIdArg,
    todo_status_arg::TodoStatusArg, tool_failure::ToolFailure,
};
use crate::{
    adapter::{
        bridge::{SessionSummary, WorkspaceRequest},
        cli::{ActionReport, CliError, ProjectView, ServiceClient, Unwatched, process},
        service::{ProcessAction, ServiceRequest, ServiceResponse},
    },
    domain::{
        coordination::{Author, KvKey, ScratchpadKey, TodoId, TodoStatus, TodoTitle},
        port::CoordinationStore,
        value::ProcessRef,
    },
};

/// Server name reported to connecting agents.
const MCP_SERVER_NAME: &str = "muster-coordination";
/// Key muster's server is registered under in an agent's MCP config; it
/// namespaces the tools an agent sees (e.g. `muster` -> `scratchpad_set`).
const MCP_CONNECTION_KEY: &str = "muster";
/// Subcommand an agent invokes to start the server.
const MCP_SUBCOMMAND: &str = "mcp";
/// Config-path flag passed to the server subcommand.
const MCP_CONFIG_FLAG: &str = "--config";
/// Server version reported to connecting agents.
const MCP_SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
/// Guidance shown to agents that connect to the coordination server.
const MCP_INSTRUCTIONS: &str = "Shared coordination state for this muster project: durable \
    scratchpad notes, structured todos, and key-values that every agent and the human read and \
    write. Use scratchpads to pass context and feedback, todos to track work, and key-values for \
    small shared facts (a chosen port, a build id). The process_* tools and session_list \
    work on the project's live processes by name, through the muster service, which starts \
    on demand. A mistake in a call, such as an unknown name, comes back as a result flagged \
    isError whose text says what to do.";
/// Entries a list tool returns when the caller does not ask for a size.
const DEFAULT_LIST_LIMIT: usize = 50;
/// Hard cap on a list tool's page, so one call cannot flood the caller's context.
const MAX_LIST_LIMIT: usize = 200;
/// Longest a lifecycle tool waits for its process to settle, under the 60
/// seconds many MCP clients give a call.
const SETTLE_BOUND: Duration = Duration::from_secs(45);
/// Acknowledgement of input written to a process's terminal.
const SENT: &str = "sent";
/// Carriage return that submits a line typed into a terminal.
const ENTER: char = '\r';
/// Key of the list in `scratchpad_list`'s result.
const SCRATCHPADS: &str = "scratchpads";
/// Key of the list in `todo_list`'s result.
const TODOS: &str = "todos";
/// Key of the list in `kv_list`'s result.
const ENTRIES: &str = "entries";
/// Key of the list in `session_list`'s result.
const SESSIONS: &str = "sessions";
/// Why a status given to `todo_set_status` was refused.
const INVALID_STATUS: &str = "is not a valid status; use pending, in-progress or done";

/// What a tool answers: its result, or why it has none.
type ToolResult = Result<CallToolResult, ToolFailure>;

/// Builds the pretty-printed JSON snippet an agent pastes into its MCP-servers
/// config to launch muster's coordination server for `project`. `executable` is
/// the muster binary path, so the entry works even when muster is not on `PATH`.
pub fn connection_config(executable: &Path, project: &Path) -> String {
    let value = serde_json::json!({
        "mcpServers": {
            MCP_CONNECTION_KEY: {
                "command": executable.display().to_string(),
                "args": [
                    MCP_SUBCOMMAND,
                    MCP_CONFIG_FLAG,
                    project.display().to_string(),
                ],
            },
        },
    });
    serde_json::to_string_pretty(&value).unwrap_or_default()
}

/// Serves the coordination store for `project` as an MCP server over stdio,
/// blocking until the client disconnects. Intended to run as a child process an
/// agent spawns, so the OS process boundary is the trust boundary; no port and no
/// token are exposed.
///
/// # Errors
/// Returns [`McpError`] if the server cannot be initialized or stops abnormally.
pub async fn serve_stdio(
    store: Box<dyn CoordinationStore + Send + Sync>,
    project: PathBuf,
    author: Author,
) -> Result<(), McpError> {
    let service = CoordinationMcp::new(store, project, author)
        .serve(stdio())
        .await
        .map_err(|error| McpError::Serve(error.to_string()))?;
    service
        .waiting()
        .await
        .map_err(|error| McpError::Wait(error.to_string()))?;
    Ok(())
}

/// Resolves a requested page size to the allowed range.
fn page_size(limit: Option<usize>) -> usize {
    limit.unwrap_or(DEFAULT_LIST_LIMIT).clamp(1, MAX_LIST_LIMIT)
}

/// MCP driving adapter exposing one project's [`CoordinationStore`] as tools.
/// Writes are attributed to `author` - the agent this server was launched for -
/// so every coordination entry an agent creates carries its identity.
pub struct CoordinationMcp {
    store: Box<dyn CoordinationStore + Send + Sync>,
    project: PathBuf,
    author: Author,
}

/// The process reference an agent typed as `name`.
///
/// # Errors
/// Returns the caller's mistake for a blank name.
fn reference(name: &str) -> Result<ProcessRef, ToolFailure> {
    ProcessRef::try_new(name.to_string())
        .map_err(|_| CliError::InvalidName(name.to_string()).into())
}

/// `report` as a tool result, flagged as an error when a process did not end
/// up as asked so the agent cannot read it as a success.
fn action_result(report: &ActionReport) -> ToolResult {
    let value = serde_json::to_value(report)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?;
    Ok(if report.failed() {
        CallToolResult::structured_error(value)
    } else {
        CallToolResult::structured(value)
    })
}

/// Types `data` into the terminal of the process `name` of `project`.
///
/// # Errors
/// Returns a [`CliError`] when the name matches no process, the process is
/// not running, or the service cannot be reached.
fn send_input(
    client: &ServiceClient,
    project: &Path,
    name: &ProcessRef,
    data: String,
) -> Result<(), CliError> {
    let name = process::resolve(&process::status(client, project)?, project, name)?;
    let request = ServiceRequest::Workspace {
        project: project.to_path_buf(),
        request: WorkspaceRequest::SendInput {
            name: name.clone(),
            data,
        },
    };
    process::expect_ok(client.send(&request)?, project, Some(&name))
}

/// This project's durable agent sessions.
///
/// # Errors
/// Returns a [`CliError`] when the service cannot be reached or cannot read
/// its session store.
fn sessions(client: &ServiceClient, project: &Path) -> Result<Vec<SessionSummary>, CliError> {
    match client.send(&ServiceRequest::Workspace {
        project: project.to_path_buf(),
        request: WorkspaceRequest::ListSessions,
    })? {
        ServiceResponse::Sessions(sessions) => Ok(sessions),
        other => Err(process::unexpected(other, project, None)),
    }
}

/// A successful tool result carrying a single text block.
fn text(body: impl Into<String>) -> CallToolResult {
    CallToolResult::success(vec![ContentBlock::text(body)])
}

/// A successful tool result carrying structured JSON (with a text fallback that
/// rmcp fills in), so an agent gets typed data rather than a prose blob.
///
/// # Errors
/// Returns an internal error if the value cannot be serialized, which cannot
/// happen for the coordination entities.
fn structured<T: Serialize>(value: &T) -> ToolResult {
    serde_json::to_value(value)
        .map(CallToolResult::structured)
        .map_err(|error| ErrorData::internal_error(error.to_string(), None).into())
}

/// `items` as a structured result holding them under `key`, since a
/// structured result is always an object.
///
/// # Errors
/// Returns an internal error if the items cannot be serialized.
fn listing<T: Serialize>(key: &str, items: &[T]) -> ToolResult {
    structured(&serde_json::json!({ key: items }))
}

#[tool_router]
impl CoordinationMcp {
    /// Builds the server over `store`, scoped to the absolute config path
    /// `project` and writing as `author`.
    pub fn new(
        store: Box<dyn CoordinationStore + Send + Sync>,
        project: PathBuf,
        author: Author,
    ) -> Self {
        Self {
            store,
            project,
            author,
        }
    }

    /// Lists every scratchpad in this project as structured objects.
    #[tool(
        description = "List the shared scratchpad notes in this project (other agents and the \
            human read and write these). Takes limit (default 50, at most 200). Returns \
            {scratchpads: [{key, author, updated_at (Unix seconds)}]}, an index without bodies \
            so a project of long notes cannot flood you. Read a body with scratchpad_get."
    )]
    async fn scratchpad_list(&self, Parameters(arg): Parameters<ListArg>) -> ToolResult {
        let notes = self.store.scratchpads(&self.project)?;
        let page: Vec<ScratchpadEntry> = notes
            .iter()
            .take(page_size(arg.limit()))
            .map(|note| {
                ScratchpadEntry::builder()
                    .key(note.key().as_ref().to_string())
                    .author(note.author().clone())
                    .updated_at(*note.updated_at())
                    .build()
            })
            .collect();
        listing(SCRATCHPADS, &page)
    }

    /// Reads one scratchpad.
    #[tool(
        description = "Read one shared scratchpad note. Takes its key. Returns {key, body, \
            author, updated_at (Unix seconds)}. A result flagged isError says no note has that key."
    )]
    async fn scratchpad_get(&self, Parameters(arg): Parameters<KeyArg>) -> ToolResult {
        let key = scratchpad_key(arg.key())?;
        match self.store.scratchpad(&self.project, &key)? {
            Some(note) => structured(&note),
            None => Err(CliError::UnknownNote(key.as_ref().to_string()).into()),
        }
    }

    /// Creates or replaces a scratchpad.
    #[tool(
        description = "Create or replace a shared scratchpad note, visible to other agents and \
            the human; use it to pass context, plans or feedback. Takes key and a markdown body. \
            Returns \"saved '<key>'\"."
    )]
    async fn scratchpad_set(&self, Parameters(arg): Parameters<ScratchpadSetArg>) -> ToolResult {
        let key = scratchpad_key(arg.key())?;
        self.store
            .set_scratchpad(&self.project, &self.author, &key, arg.body())?;
        Ok(text(format!("saved '{}'", key.as_ref())))
    }

    /// Deletes a scratchpad.
    #[tool(
        description = "Delete a scratchpad note. Takes its key. Returns \"deleted '<key>'\", or \
            \"no scratchpad '<key>'\" when there was none."
    )]
    async fn scratchpad_delete(&self, Parameters(arg): Parameters<KeyArg>) -> ToolResult {
        let key = scratchpad_key(arg.key())?;
        let removed = self.store.delete_scratchpad(&self.project, &key)?;
        Ok(text(if removed {
            format!("deleted '{}'", key.as_ref())
        } else {
            format!("no scratchpad '{}'", key.as_ref())
        }))
    }

    /// Lists every todo as structured objects, including dependencies.
    #[tool(
        description = "List the shared todos in this project. Takes limit (default 50, at most \
            200). Returns {todos: [{id, title, status (pending|in-progress|done), deps (ids this \
            todo depends on), author, updated_at (Unix seconds)}]}."
    )]
    async fn todo_list(&self, Parameters(arg): Parameters<ListArg>) -> ToolResult {
        let todos = self.store.todos(&self.project)?;
        let page: Vec<_> = todos.into_iter().take(page_size(arg.limit())).collect();
        listing(TODOS, &page)
    }

    /// Reads one todo, including its dependencies.
    #[tool(
        description = "Read one todo. Takes its id. Returns {id, title, status, deps, author, \
            updated_at}. A result flagged isError says no todo has that id."
    )]
    async fn todo_get(&self, Parameters(arg): Parameters<TodoIdArg>) -> ToolResult {
        let id = todo_id(arg.id())?;
        match self.store.todo(&self.project, &id)? {
            Some(todo) => structured(&todo),
            None => Err(CliError::UnknownTodo(id.as_ref().to_string()).into()),
        }
    }

    /// Adds a new pending todo, returning the created todo.
    #[tool(
        description = "Add a new shared, pending todo. Takes title and deps, the ids of todos \
            this one depends on (optional). Returns the created todo {id, title, status, deps, \
            author, updated_at}."
    )]
    async fn todo_add(&self, Parameters(arg): Parameters<TodoAddArg>) -> ToolResult {
        let title = TodoTitle::try_new(arg.title().clone())
            .map_err(|_| CliError::InvalidTodoTitle(arg.title().clone()))?;
        let deps = arg
            .deps()
            .iter()
            .map(|dep| todo_id(dep))
            .collect::<Result<Vec<_>, _>>()?;
        let created = self
            .store
            .add_todo(&self.project, &self.author, &title, &deps)?;
        structured(&created)
    }

    /// Sets a todo's lifecycle status.
    #[tool(
        description = "Set a todo's status. Takes its id and status (pending|in-progress|done). \
            Returns \"<id> is <status>\". A result flagged isError says the id or status is wrong."
    )]
    async fn todo_set_status(&self, Parameters(arg): Parameters<TodoStatusArg>) -> ToolResult {
        let id = todo_id(arg.id())?;
        let status = arg
            .status()
            .parse::<TodoStatus>()
            .map_err(|_| CliError::Usage(format!("'{}' {INVALID_STATUS}", arg.status())))?;
        if self
            .store
            .set_todo_status(&self.project, &self.author, &id, status)?
        {
            Ok(text(format!("{id} is {status}")))
        } else {
            Err(CliError::UnknownTodo(id.as_ref().to_string()).into())
        }
    }

    /// Deletes a todo.
    #[tool(
        description = "Delete a todo. Takes its id. Returns \"deleted '<id>'\", or \"no todo \
            '<id>'\" when there was none."
    )]
    async fn todo_delete(&self, Parameters(arg): Parameters<TodoIdArg>) -> ToolResult {
        let id = todo_id(arg.id())?;
        let removed = self.store.delete_todo(&self.project, &id)?;
        Ok(text(if removed {
            format!("deleted '{id}'")
        } else {
            format!("no todo '{id}'")
        }))
    }

    /// Lists every key-value entry as structured objects.
    #[tool(
        description = "List the shared key-value entries in this project. Takes limit (default \
            50, at most 200). Returns {entries: [{key, value, author, updated_at (Unix seconds)}]}."
    )]
    async fn kv_list(&self, Parameters(arg): Parameters<ListArg>) -> ToolResult {
        let entries = self.store.values(&self.project)?;
        let page: Vec<_> = entries.into_iter().take(page_size(arg.limit())).collect();
        listing(ENTRIES, &page)
    }

    /// Reads one key-value entry.
    #[tool(
        description = "Read one shared key-value entry. Takes its key. Returns {key, value, \
            author, updated_at}. A result flagged isError says no entry has that key."
    )]
    async fn kv_get(&self, Parameters(arg): Parameters<KeyArg>) -> ToolResult {
        let key = kv_key(arg.key())?;
        match self.store.value(&self.project, &key)? {
            Some(entry) => structured(&entry),
            None => Err(CliError::UnknownValue(key.as_ref().to_string()).into()),
        }
    }

    /// Creates or replaces a key-value entry.
    #[tool(
        description = "Create or replace a shared key-value entry, for small facts other agents \
            need: a chosen port, a build id, a flag. Takes key and value. Returns \"saved '<key>'\"."
    )]
    async fn kv_set(&self, Parameters(arg): Parameters<KvSetArg>) -> ToolResult {
        let key = kv_key(arg.key())?;
        self.store
            .set_value(&self.project, &self.author, &key, arg.value())?;
        Ok(text(format!("saved '{}'", key.as_ref())))
    }

    /// Deletes a key-value entry.
    #[tool(
        description = "Delete a key-value entry. Takes its key. Returns \"deleted '<key>'\", or \
            \"no value '<key>'\" when there was none."
    )]
    async fn kv_delete(&self, Parameters(arg): Parameters<KeyArg>) -> ToolResult {
        let key = kv_key(arg.key())?;
        let removed = self.store.delete_value(&self.project, &key)?;
        Ok(text(if removed {
            format!("deleted '{}'", key.as_ref())
        } else {
            format!("no value '{}'", key.as_ref())
        }))
    }

    /// Lists this project's processes.
    #[tool(
        description = "List this project's processes (agents, terminals and commands) in \
            muster.yml order. Takes no arguments. Returns {project, processes}, where project is \
            the config path and each process is {name, kind (agent|terminal|command), state \
            (pending|starting|running|paused|stopping|exited|crashed|restarting|unsupervised), \
            activity (idle|working|awaiting-input), pid, uptime (whole seconds), restarts, \
            cpu_percent, memory_bytes, attached (how many clients are attached to type into its \
            terminal), last_exit ({status: exit code or null, signal: the signal that ended it by \
            its lowercase POSIX name such as sigterm or sigkill, signal-<number> for one with no \
            POSIX name, or null}, while it is not running), stop_grace (whole \
            seconds a stop waits before it kills)}. A null field has no value right now. Use a \
            process's name with the other process_* tools."
    )]
    async fn process_list(&self) -> ToolResult {
        let status = self.ask(process::status).await?;
        structured(&ProjectView::from(&status))
    }

    /// Reads the visible screen of a process's terminal.
    #[tool(
        description = "Read what a process's terminal shows right now - what a dev server, test \
            run or agent last printed. Takes the process's name. Returns the screen as text \
            without its trailing blank lines, empty when the process has not started. An unknown \
            name is a result flagged isError that lists the valid names."
    )]
    async fn process_output(&self, Parameters(arg): Parameters<NameArg>) -> ToolResult {
        let name = reference(arg.name())?;
        let screen = self
            .ask(move |client, project| {
                process::logs(client, project, process::named(project, &name), true)
            })
            .await?;
        Ok(text(screen.text().trim_end()))
    }

    /// Lists this project's durable agent sessions.
    #[tool(
        description = "List this project's agent sessions. Takes no arguments. Returns \
            {sessions: [{name, tool (claude|codex|gemini|amp|opencode|copilot|kimi|custom), state \
            (pending|open|closed), resumable}]}; resumable means the agent's conversation was \
            captured, so it can be resumed rather than started fresh."
    )]
    async fn session_list(&self) -> ToolResult {
        listing(SESSIONS, &self.ask(sessions).await?)
    }

    /// Starts a process.
    #[tool(
        description = "Start a process unless it is already running. Takes the process's name. \
            Waits for it to settle, at most 45 seconds, then returns {project, action, processes: \
            [{name, outcome, reason, status}], notices, other_notices, wait_ended}: status is its \
            process_list entry now, notices explain a failure, and wait_ended is settled, \
            timed-out (muster's own wait for this action expired) or cut-short (the 45 second \
            bound expired first). outcome is one of started, already-running, exited (it ran and \
            exited), crashed, unsupervised (an earlier run may still be running, so it was not \
            started), not-started, still-starting or still-stopping (it had not settled when the \
            wait ended), or gone. The result is flagged isError when the outcome is anything but \
            started or already-running, except that still-starting or still-stopping after a \
            cut-short wait is not an error: the process is on its way, so follow it with \
            process_list."
    )]
    async fn process_start(&self, Parameters(arg): Parameters<NameArg>) -> ToolResult {
        self.control(arg.name(), ProcessAction::Start).await
    }

    /// Stops a process.
    #[tool(
        description = "Stop a process without its restart policy respawning it. Takes the \
            process's name. The stop is graceful: it waits out the process's stop grace, and \
            this tool waits at most 45 seconds in all. Returns the same report as process_start, \
            whose outcome is one of stopped, was-not-running, still-stopping (it had not exited \
            when the wait ended) or still-running. The result is flagged isError when the \
            outcome is anything but stopped or was-not-running, except that still-stopping after \
            a cut-short wait is not an error: follow it with process_list."
    )]
    async fn process_stop(&self, Parameters(arg): Parameters<NameArg>) -> ToolResult {
        self.control(arg.name(), ProcessAction::Stop).await
    }

    /// Restarts a process.
    #[tool(
        description = "Restart a process, whatever its restart policy, starting it if it was not \
            running. Takes the process's name. Waits for it to settle, at most 45 seconds, then \
            returns the same report as process_start, whose outcome is one of restarted, \
            still-running (the old run did not end), exited, crashed, unsupervised, not-started, \
            still-starting or still-stopping (it had not settled when the wait ended), or gone. \
            The result is flagged isError when the outcome is anything but restarted, except \
            that still-starting or still-stopping after a cut-short wait is not an error: follow \
            it with process_list."
    )]
    async fn process_restart(&self, Parameters(arg): Parameters<NameArg>) -> ToolResult {
        self.control(arg.name(), ProcessAction::Restart).await
    }

    /// Types text into a process's terminal.
    #[tool(
        description = "Type text into a process's terminal as if a person typed it - to answer \
            a prompt, run a command in a shell, or unblock a waiting process. Takes the process's \
            name, the text, and enter (default false); enter=true submits the line. Returns \
            \"sent\". A process that is not running, or an unknown name, is a result flagged \
            isError that says what to do."
    )]
    async fn process_send(&self, Parameters(arg): Parameters<SendInputArg>) -> ToolResult {
        let name = reference(arg.name())?;
        let data = if arg.enter() {
            format!("{}{ENTER}", arg.text())
        } else {
            arg.text().clone()
        };
        self.ask(move |client, project| send_input(client, project, &name, data))
            .await?;
        Ok(text(SENT))
    }
}

impl CoordinationMcp {
    /// Runs `query` against this project through the service, starting or
    /// replacing the service as the CLI does. It runs on a blocking thread,
    /// since a query may wait for a process to settle.
    ///
    /// # Errors
    /// Returns the caller's mistake, such as an unknown name, or the fault of
    /// a service that cannot be reached.
    async fn ask<T, Query>(&self, query: Query) -> Result<T, ToolFailure>
    where
        T: Send + 'static,
        Query: FnOnce(&ServiceClient, &Path) -> Result<T, CliError> + Send + 'static,
    {
        let project = self.project.clone();
        task::spawn_blocking(move || {
            ServiceClient::connect().and_then(|client| query(&client, &project))
        })
        .await
        .map_err(|error| ErrorData::internal_error(error.to_string(), None))?
        .map_err(ToolFailure::from)
    }

    /// Applies `action` to the process an agent named `name` and reports what
    /// became of it once it settled, or once [`SETTLE_BOUND`] passed.
    ///
    /// # Errors
    /// Returns the caller's mistake for a blank or unknown name, or the fault
    /// of a service that cannot be reached.
    async fn control(&self, name: &str, action: ProcessAction) -> ToolResult {
        let name = reference(name)?;
        let report = self
            .ask(move |client, project| {
                process::control_within(
                    client,
                    project,
                    process::named(project, &name),
                    action,
                    SETTLE_BOUND,
                    &mut Unwatched,
                )
            })
            .await?;
        action_result(&report)
    }
}

/// The scratchpad key an agent typed as `key`.
///
/// # Errors
/// Returns the caller's mistake for a blank key.
fn scratchpad_key(key: &str) -> Result<ScratchpadKey, ToolFailure> {
    ScratchpadKey::try_new(key.to_string())
        .map_err(|_| CliError::InvalidNoteKey(key.to_string()).into())
}

/// The todo id an agent typed as `id`.
///
/// # Errors
/// Returns the caller's mistake for a blank id.
fn todo_id(id: &str) -> Result<TodoId, ToolFailure> {
    TodoId::try_new(id.to_string()).map_err(|_| CliError::InvalidTodoId(id.to_string()).into())
}

/// The key-value key an agent typed as `key`.
///
/// # Errors
/// Returns the caller's mistake for a blank key.
fn kv_key(key: &str) -> Result<KvKey, ToolFailure> {
    KvKey::try_new(key.to_string()).map_err(|_| CliError::InvalidKvKey(key.to_string()).into())
}

#[tool_handler]
impl ServerHandler for CoordinationMcp {
    fn get_info(&self) -> ServerInfo {
        let mut implementation = Implementation::default();
        implementation.name = MCP_SERVER_NAME.to_string();
        implementation.version = MCP_SERVER_VERSION.to_string();
        let mut info = ServerInfo::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info.server_info = implementation;
        info.instructions = Some(MCP_INSTRUCTIONS.to_string());
        info
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::{
        adapter::cli::{Action, Outcome, ProcessOutcome, WaitEnd},
        domain::{
            config::ConfigError,
            coordination::{Author, KeyValue, Scratchpad, Todo},
            value::ProcessName,
        },
    };

    /// The project every test serves.
    const PROJECT: &str = "/repo/muster.yml";

    /// A server for [`PROJECT`] over an in-memory store.
    fn server() -> CoordinationMcp {
        CoordinationMcp::new(
            Box::new(FakeStore::default()),
            PathBuf::from(PROJECT),
            Author::agent("test"),
        )
    }

    /// An in-memory, thread-safe coordination store recording scratchpads.
    #[derive(Default)]
    struct FakeStore {
        notes: Mutex<Vec<Scratchpad>>,
    }

    impl CoordinationStore for FakeStore {
        fn scratchpads(&self, _project: &Path) -> Result<Vec<Scratchpad>, ConfigError> {
            Ok(self.notes.lock().unwrap().clone())
        }

        fn scratchpad(
            &self,
            _project: &Path,
            key: &ScratchpadKey,
        ) -> Result<Option<Scratchpad>, ConfigError> {
            Ok(self
                .notes
                .lock()
                .unwrap()
                .iter()
                .find(|note| note.key() == key)
                .cloned())
        }

        fn set_scratchpad(
            &self,
            _project: &Path,
            author: &Author,
            key: &ScratchpadKey,
            body: &str,
        ) -> Result<(), ConfigError> {
            let note = Scratchpad::builder()
                .key(key.clone())
                .body(body.to_owned())
                .author(author.clone())
                .updated_at(0)
                .build();
            let mut notes = self.notes.lock().unwrap();
            notes.retain(|existing| existing.key() != key);
            notes.push(note);
            Ok(())
        }

        fn delete_scratchpad(
            &self,
            _project: &Path,
            _key: &ScratchpadKey,
        ) -> Result<bool, ConfigError> {
            unreachable!("this test never deletes scratchpads")
        }

        fn todos(&self, _project: &Path) -> Result<Vec<Todo>, ConfigError> {
            unreachable!("this test never touches todos")
        }

        fn todo(&self, _project: &Path, _id: &TodoId) -> Result<Option<Todo>, ConfigError> {
            unreachable!("this test never touches todos")
        }

        fn add_todo(
            &self,
            _project: &Path,
            _author: &Author,
            _title: &TodoTitle,
            _deps: &[TodoId],
        ) -> Result<Todo, ConfigError> {
            unreachable!("this test never touches todos")
        }

        fn set_todo_status(
            &self,
            _project: &Path,
            _author: &Author,
            _id: &TodoId,
            _status: TodoStatus,
        ) -> Result<bool, ConfigError> {
            unreachable!("this test never touches todos")
        }

        fn delete_todo(&self, _project: &Path, _id: &TodoId) -> Result<bool, ConfigError> {
            unreachable!("this test never touches todos")
        }

        fn values(&self, _project: &Path) -> Result<Vec<KeyValue>, ConfigError> {
            unreachable!("this test never touches key-values")
        }

        fn value(&self, _project: &Path, _key: &KvKey) -> Result<Option<KeyValue>, ConfigError> {
            unreachable!("this test never touches key-values")
        }

        fn set_value(
            &self,
            _project: &Path,
            _author: &Author,
            _key: &KvKey,
            _value: &str,
        ) -> Result<(), ConfigError> {
            unreachable!("this test never touches key-values")
        }

        fn delete_value(&self, _project: &Path, _key: &KvKey) -> Result<bool, ConfigError> {
            unreachable!("this test never touches key-values")
        }

        fn import(
            &self,
            _project: &Path,
            _scratchpads: &[Scratchpad],
            _todos: &[Todo],
            _values: &[KeyValue],
        ) -> Result<(), ConfigError> {
            unreachable!("these tests never import")
        }
    }

    /// A start report for `api` whose outcome is `outcome` and whose wait
    /// ended as `wait_ended`.
    fn report(outcome: Outcome, wait_ended: WaitEnd) -> ActionReport {
        ActionReport::builder()
            .project(PathBuf::from(PROJECT))
            .action(Action::Start)
            .processes(vec![
                ProcessOutcome::builder()
                    .name(ProcessName::try_new("api").unwrap())
                    .outcome(outcome)
                    .status(None)
                    .build(),
            ])
            .notices(Vec::new())
            .other_notices(Vec::new())
            .wait_ended(wait_ended)
            .build()
    }

    /// Builds a single-threaded runtime for the async tool calls.
    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
    }

    /// The text of a tool result's first content block.
    fn result_text(result: &CallToolResult) -> String {
        match &result.content[0] {
            ContentBlock::Text(text) => text.text.clone(),
            other => panic!("expected text content, got {other:?}"),
        }
    }

    /// What the agent receives for `result`.
    fn answered(result: ToolResult) -> Result<CallToolResult, ErrorData> {
        result.or_else(ToolFailure::answer)
    }

    /// The scratchpad `key` with `body`, to set.
    fn note(key: &str, body: &str) -> Parameters<ScratchpadSetArg> {
        Parameters(
            ScratchpadSetArg::builder()
                .key(key.to_string())
                .body(body.to_string())
                .build(),
        )
    }

    /// A `scratchpad_set` tool call writes through to the store, and
    /// `scratchpad_list` reads it back as structured data - the store-backed
    /// round trip an agent depends on.
    #[test]
    fn a_set_tool_call_writes_through_to_the_store() {
        let server = server();

        runtime().block_on(async {
            let saved = server
                .scratchpad_set(note("plan", "ship it"))
                .await
                .unwrap();
            assert_eq!(result_text(&saved), "saved 'plan'");

            let listed = server
                .scratchpad_list(Parameters(ListArg::builder().build()))
                .await
                .unwrap();
            let index = listed
                .structured_content
                .expect("list returns structured data");
            let notes = &index[SCRATCHPADS];
            assert_eq!(notes[0]["key"], "plan");
            assert_eq!(notes[0]["author"], "test");
            assert!(
                notes[0].get("updated_at").is_some(),
                "updated_at is surfaced"
            );
            assert!(
                notes[0].get("body").is_none(),
                "the index omits bodies so a long note cannot flood the caller"
            );

            let fetched = server
                .scratchpad_get(Parameters(
                    KeyArg::builder().key("plan".to_string()).build(),
                ))
                .await
                .unwrap();
            assert_eq!(
                fetched.structured_content.expect("structured note")["body"],
                "ship it"
            );
        });
    }

    /// A blank key is the caller's mistake, answered as an error result
    /// before the store is touched.
    #[test]
    fn a_blank_key_is_an_error_result() {
        let server = server();

        runtime().block_on(async {
            let result = answered(server.scratchpad_set(note("   ", "x")).await).unwrap();
            assert_eq!(result.is_error, Some(true));
            assert_eq!(
                result.structured_content.expect("structured")["error"]["kind"],
                "invalid-input"
            );
        });
    }

    /// A blank process name is the caller's mistake, answered before the
    /// service is asked.
    #[test]
    fn a_blank_process_name_is_an_error_result() {
        let server = server();
        runtime().block_on(async {
            let result = answered(
                server
                    .process_output(Parameters(
                        NameArg::builder().name("   ".to_string()).build(),
                    ))
                    .await,
            )
            .unwrap();
            assert_eq!(result.is_error, Some(true));
        });
    }

    /// The connection snippet is valid JSON that launches this binary with the
    /// project's config path under the muster server key.
    #[test]
    fn connection_config_is_valid_and_points_at_this_binary() {
        let json = connection_config(
            Path::new("/usr/local/bin/muster"),
            Path::new("/repo/muster.yml"),
        );
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        let entry = &parsed["mcpServers"][MCP_CONNECTION_KEY];
        assert_eq!(entry["command"], "/usr/local/bin/muster");
        assert_eq!(
            entry["args"],
            serde_json::json!(["mcp", "--config", "/repo/muster.yml"])
        );
    }

    /// A page size defaults, clamps to the cap, and never yields an empty page.
    #[test]
    fn page_size_defaults_and_clamps() {
        assert_eq!(page_size(None), DEFAULT_LIST_LIMIT);
        assert_eq!(page_size(Some(5)), 5);
        assert_eq!(page_size(Some(0)), 1, "a zero page would return nothing");
        assert_eq!(page_size(Some(10_000)), MAX_LIST_LIMIT);
    }

    /// `scratchpad_list` returns at most the requested page, so a project with
    /// many notes cannot flood the caller in one call.
    #[test]
    fn scratchpad_list_honours_the_limit() {
        let server = server();
        runtime().block_on(async {
            for key in ["a", "b", "c"] {
                server.scratchpad_set(note(key, "x")).await.unwrap();
            }
            let listed = server
                .scratchpad_list(Parameters(ListArg::builder().limit(Some(2)).build()))
                .await
                .unwrap();
            let index = listed.structured_content.expect("structured index");
            assert_eq!(index[SCRATCHPADS].as_array().expect("an array").len(), 2);
        });
    }

    /// A lifecycle report is structured data, flagged as an error when the
    /// process did not end up as asked so an agent cannot read it as done.
    #[test]
    fn a_failed_action_report_is_an_error_result() {
        let started = action_result(&report(Outcome::Started, WaitEnd::Settled)).unwrap();
        assert_eq!(started.is_error, Some(false));
        let content = started.structured_content.expect("a structured report");
        assert_eq!(content["action"], "start");
        assert_eq!(content["processes"][0]["outcome"], "started");

        let crashed = action_result(&report(Outcome::Crashed, WaitEnd::Settled)).unwrap();
        assert_eq!(crashed.is_error, Some(true));
        assert_eq!(
            crashed.structured_content.expect("a structured report")["processes"][0]["outcome"],
            "crashed"
        );
    }

    /// A process still stopping when this server's bound cut the wait short
    /// is on its way, not failed; one still stopping when the action's own
    /// timeout expired failed to settle.
    #[test]
    fn a_wait_cut_short_is_no_error_and_a_timed_out_one_is() {
        let cut = action_result(&report(Outcome::StillStopping, WaitEnd::CutShort)).unwrap();
        assert_eq!(cut.is_error, Some(false));
        let content = cut.structured_content.expect("a structured report");
        assert_eq!(content["wait_ended"], "cut-short");
        assert_eq!(content["processes"][0]["outcome"], "still-stopping");

        let timed_out = action_result(&report(Outcome::StillStopping, WaitEnd::TimedOut)).unwrap();
        assert_eq!(timed_out.is_error, Some(true));
        assert_eq!(
            timed_out.structured_content.expect("a structured report")["wait_ended"],
            "timed-out"
        );
        assert_eq!(
            action_result(&report(Outcome::Crashed, WaitEnd::CutShort))
                .unwrap()
                .is_error,
            Some(true),
            "a settled failure stays a failure whatever cut the wait"
        );
    }

    /// A list is held under its key, since a structured result is always an
    /// object.
    #[test]
    fn a_listing_is_an_object() {
        let listed = listing(SESSIONS, &Vec::<SessionSummary>::new()).unwrap();
        assert_eq!(
            listed.structured_content,
            Some(serde_json::json!({ "sessions": [] }))
        );
    }

    /// The advertised server info names the coordination server and enables the
    /// tools capability.
    #[test]
    fn server_info_advertises_tools() {
        let server = server();
        let info = server.get_info();
        assert_eq!(info.server_info.name, MCP_SERVER_NAME);
        assert!(info.capabilities.tools.is_some());
    }
}
